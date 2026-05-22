use anyhow::Result;
use bytes::Bytes;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures::stream::BoxStream;
use rubato::Resampler;

const TARGET_SAMPLE_RATE: u32 = 16000;

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

    pub fn start(&self) -> Result<(cpal::Stream, BoxStream<'static, Result<Bytes>>)> {
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
        tracing::info!(
            "device sample rate: {}Hz, target: {}Hz",
            input_sample_rate,
            self.target_sample_rate
        );

        let config = supported_config.config();

        let needs_resampling = input_sample_rate != self.target_sample_rate;
        let resample_ratio = self.target_sample_rate as f64 / input_sample_rate as f64;

        let (raw_tx, raw_rx) = tokio::sync::mpsc::channel::<Vec<f32>>(32);

        let stream = build_input_stream(&device, &config, raw_tx)?;
        stream.play()?;

        tracing::info!("microphone capture started, press Ctrl+C to stop");

        let (out_tx, out_rx) = tokio::sync::mpsc::channel::<Result<Bytes>>(8);

        tokio::spawn(async move {
            use tokio_stream::StreamExt as _;
            let mut raw_stream = tokio_stream::wrappers::ReceiverStream::new(raw_rx);

            let mut resampler: Option<rubato::Async<f64>> = if needs_resampling {
                let sinc_params = rubato::SincInterpolationParameters {
                    sinc_len: 256,
                    f_cutoff: rubato::calculate_cutoff(
                        256,
                        rubato::WindowFunction::BlackmanHarris2,
                    ),
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

            let mut input_buffer: Vec<f64> = Vec::new();

            while let Some(chunk) = raw_stream.next().await {
                if let Some(ref mut resampler) = resampler {
                    input_buffer.extend(chunk.iter().map(|&s| f64::from(s)));

                    let needed = resampler.input_frames_next();
                    while input_buffer.len() >= needed {
                        let buf = audioadapter_buffers::direct::InterleavedSlice::new(
                            &input_buffer[..needed],
                            1,
                            needed,
                        )
                        .unwrap();
                        match resampler.process(&buf, 0, None) {
                            Ok(output) => {
                                let samples = output.take_data();
                                let pcm = f64_to_pcm_bytes(&samples);
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
                        input_buffer.drain(..needed);
                    }
                } else {
                    let pcm = f32_to_pcm_bytes(&chunk);
                    if out_tx.send(Ok(pcm)).await.is_err() {
                        return;
                    }
                }
            }

            // Flush remaining samples
            if !input_buffer.is_empty() {
                if let Some(ref mut resampler) = resampler {
                    let buf = audioadapter_buffers::direct::InterleavedSlice::new(
                        &input_buffer,
                        1,
                        input_buffer.len(),
                    )
                    .unwrap();
                    if let Ok(output) = resampler.process(&buf, 0, None) {
                        let samples = output.take_data();
                        let pcm = f64_to_pcm_bytes(&samples);
                        let _ = out_tx.send(Ok(pcm)).await;
                    }
                } else {
                    // Non-resampling path doesn't buffer, so nothing to flush
                }
            }
        });

        let stream_out =
            futures::StreamExt::boxed(tokio_stream::wrappers::ReceiverStream::new(out_rx));
        Ok((stream, stream_out))
    }
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

fn f64_to_pcm_bytes(samples: &[f64]) -> Bytes {
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for &sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
        pcm.extend_from_slice(&value.to_le_bytes());
    }
    Bytes::from(pcm)
}

fn f32_to_pcm_bytes(samples: &[f32]) -> Bytes {
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for &sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
        pcm.extend_from_slice(&value.to_le_bytes());
    }
    Bytes::from(pcm)
}
