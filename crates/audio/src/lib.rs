use std::collections::VecDeque;

use anyhow::Result;
use bytes::Bytes;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures::stream::BoxStream;
use rubato::Resampler;

const TARGET_SAMPLE_RATE: u32 = 16000;

struct DeviceInfo {
    channels: usize,
    needs_resampling: bool,
    resample_ratio: f64,
}

pub struct MicrophoneCapture {
    device_name: Option<String>,
    target_sample_rate: u32,
}

impl MicrophoneCapture {
    pub fn new(device_name: Option<String>) -> Self {
        Self {
            device_name,
            target_sample_rate: TARGET_SAMPLE_RATE,
        }
    }

    pub fn list_devices() -> Result<Vec<String>> {
        let host = cpal::default_host();
        let devices: Vec<String> = host
            .input_devices()?
            .filter_map(|d| d.description().ok().map(|desc| desc.to_string()))
            .collect();
        Ok(devices)
    }

    /// Start continuous streaming capture.
    pub fn start(&self) -> Result<(cpal::Stream, BoxStream<'static, Result<Bytes>>)> {
        let (stream, raw_rx, info) = self.setup_device_and_stream()?;
        tracing::info!("microphone capture started, press Ctrl+C to stop");
        let (out_tx, out_rx) = tokio::sync::mpsc::channel::<Result<Bytes>>(32);
        spawn_resample_task(raw_rx, info, out_tx);
        let stream_out =
            futures::StreamExt::boxed(tokio_stream::wrappers::ReceiverStream::new(out_rx));
        Ok((stream, stream_out))
    }

    /// Start a session-mode recording. Call `SessionRecorder::stop()` to end.
    pub fn record_session(&self) -> Result<SessionRecorder> {
        let (stream, raw_rx, info) = self.setup_device_and_stream()?;
        tracing::info!("session recording started");
        let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<Result<Bytes>>(32);
        spawn_resample_task(raw_rx, info, out_tx);

        // Spawn an accumulator task that continuously drains the output channel
        // into a Vec, preventing backpressure on the resampling task.
        let accumulator = tokio::spawn(async move {
            let mut pcm = Vec::new();
            while let Some(chunk) = out_rx.recv().await {
                match chunk {
                    Ok(data) => pcm.extend_from_slice(&data),
                    Err(e) => return Err(e),
                }
            }
            Ok(pcm)
        });

        Ok(SessionRecorder {
            stream,
            accumulator,
        })
    }

    fn setup_device_and_stream(
        &self,
    ) -> Result<(
        cpal::Stream,
        tokio::sync::mpsc::Receiver<Vec<f32>>,
        DeviceInfo,
    )> {
        let host = cpal::default_host();
        let device = match &self.device_name {
            Some(name) => host
                .input_devices()?
                .find(|d| {
                    d.description()
                        .map(|desc| desc.name().contains(name))
                        .unwrap_or(false)
                })
                .ok_or_else(|| anyhow::anyhow!("audio device not found: {}", name))?,
            None => host
                .default_input_device()
                .ok_or_else(|| anyhow::anyhow!("no default audio input device"))?,
        };

        let device_desc = device
            .description()
            .map(|d| d.to_string())
            .unwrap_or_default();
        tracing::info!("using audio device: {}", device_desc);

        let supported_config = device.default_input_config()?;
        let input_sample_rate: u32 = supported_config.sample_rate();
        let channels = supported_config.channels() as usize;
        tracing::info!(
            "device sample rate: {}Hz, channels: {}, target: {}Hz",
            input_sample_rate,
            channels,
            self.target_sample_rate
        );

        let config = supported_config.config();

        let needs_resampling = input_sample_rate != self.target_sample_rate;
        let resample_ratio = self.target_sample_rate as f64 / input_sample_rate as f64;

        let (raw_tx, raw_rx) = tokio::sync::mpsc::channel::<Vec<f32>>(32);

        let stream = build_input_stream(&device, &config, raw_tx)?;
        stream.play()?;

        let info = DeviceInfo {
            channels,
            needs_resampling,
            resample_ratio,
        };

        Ok((stream, raw_rx, info))
    }
}

pub struct SessionRecorder {
    stream: cpal::Stream,
    accumulator: tokio::task::JoinHandle<Result<Vec<u8>>>,
}

impl SessionRecorder {
    /// Stop recording and return all captured PCM audio (16kHz 16-bit mono).
    pub async fn stop(self) -> Result<Vec<u8>> {
        // Dropping the cpal stream stops audio capture. The cpal callback
        // closure is dropped along with the sender side of the raw channel,
        // so the resampling task will eventually see the raw stream end,
        // flush remaining samples, and close the output channel.
        // The accumulator task will then finish and return all collected PCM.
        drop(self.stream);
        self.accumulator.await?
    }
}

fn spawn_resample_task(
    raw_rx: tokio::sync::mpsc::Receiver<Vec<f32>>,
    info: DeviceInfo,
    out_tx: tokio::sync::mpsc::Sender<Result<Bytes>>,
) {
    let DeviceInfo {
        channels,
        needs_resampling,
        resample_ratio,
    } = info;

    tokio::spawn(async move {
        use tokio_stream::StreamExt as _;
        let mut raw_stream = tokio_stream::wrappers::ReceiverStream::new(raw_rx);

        let mut resampler: Option<rubato::Async<f64>> = if needs_resampling {
            let sinc_params = rubato::SincInterpolationParameters {
                sinc_len: 256,
                f_cutoff: rubato::calculate_cutoff(256, rubato::WindowFunction::BlackmanHarris2),
                oversampling_factor: 256,
                interpolation: rubato::SincInterpolationType::Linear,
                window: rubato::WindowFunction::BlackmanHarris2,
            };
            match rubato::Async::new_sinc(
                resample_ratio,
                2.0,
                &sinc_params,
                1024,
                1,
                rubato::FixedAsync::Output,
            ) {
                Ok(r) => Some(r),
                Err(e) => {
                    let _ = out_tx
                        .send(Err(anyhow::anyhow!("resampler init failed: {}", e)))
                        .await;
                    return;
                }
            }
        } else {
            None
        };

        let mut input_buffer: VecDeque<f64> = VecDeque::new();

        while let Some(chunk) = raw_stream.next().await {
            let mono = downmix_to_mono(&chunk, channels);
            if let Some(ref mut resampler) = resampler {
                input_buffer.extend(mono.iter().map(|&s| f64::from(s)));

                let needed = resampler.input_frames_next();
                while input_buffer.len() >= needed {
                    let consumed = needed;
                    let buf_chunk: Vec<f64> = input_buffer.drain(..consumed).collect();
                    let buf = audioadapter_buffers::direct::InterleavedSlice::new(
                        &buf_chunk, 1, consumed,
                    );
                    let buf = match buf {
                        Ok(b) => b,
                        Err(e) => {
                            if out_tx
                                .send(Err(anyhow::anyhow!("buffer creation failed: {}", e)))
                                .await
                                .is_err()
                            {
                                return;
                            }
                            continue;
                        }
                    };
                    match resampler.process(&buf, 0, None) {
                        Ok(output) => {
                            let samples = output.take_data();
                            let pcm = float_to_pcm_bytes(&samples);
                            if out_tx.send(Ok(pcm)).await.is_err() {
                                return;
                            }
                        }
                        Err(e) => {
                            if out_tx
                                .send(Err(anyhow::anyhow!("resample error: {}", e)))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                }
            } else {
                let pcm = float_to_pcm_bytes(&mono);
                if out_tx.send(Ok(pcm)).await.is_err() {
                    return;
                }
            }
        }

        // Flush remaining samples
        if !input_buffer.is_empty()
            && let Some(ref mut resampler) = resampler
        {
            let buf_chunk: Vec<f64> = input_buffer.drain(..).collect();
            let buf =
                audioadapter_buffers::direct::InterleavedSlice::new(&buf_chunk, 1, buf_chunk.len());
            if let Ok(buf) = buf
                && let Ok(output) = resampler.process(&buf, 0, None)
            {
                let samples = output.take_data();
                let pcm = float_to_pcm_bytes(&samples);
                let _ = out_tx.send(Ok(pcm)).await;
            }
        }
    });
}

fn downmix_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels == 1 {
        return interleaved.to_vec();
    }
    let frames = interleaved.len() / channels;
    let mut mono = Vec::with_capacity(frames);
    for frame_idx in 0..frames {
        let offset = frame_idx * channels;
        let sum: f32 = interleaved[offset..offset + channels].iter().sum();
        mono.push(sum / channels as f32);
    }
    mono
}

fn build_input_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    tx: tokio::sync::mpsc::Sender<Vec<f32>>,
) -> Result<cpal::Stream> {
    let stream = device.build_input_stream(
        config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            let _ = tx.try_send(data.to_vec());
        },
        |err| {
            tracing::error!("audio capture error: {}", err);
        },
        None,
    )?;
    Ok(stream)
}

fn float_to_pcm_bytes<T>(samples: &[T]) -> Bytes
where
    T: Into<f64> + Copy,
{
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for &sample in samples {
        let value = (Into::<f64>::into(sample).clamp(-1.0, 1.0) * 32767.0) as i16;
        pcm.extend_from_slice(&value.to_le_bytes());
    }
    Bytes::from(pcm)
}
