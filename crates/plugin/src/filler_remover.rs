use anyhow::Result;
use async_trait::async_trait;

use crate::{Plugin, PluginContext};

/// Removes Chinese filler words (口头语).
pub struct FillerRemover;

#[async_trait]
impl Plugin for FillerRemover {
    fn name(&self) -> &str {
        "filler_remover"
    }

    async fn process(&self, text: &str, _ctx: &PluginContext) -> Result<String> {
        let fillers = ["嗯", "呃", "啊", "那个", "就是说", "然后呢"];
        let mut result = text.to_string();
        for filler in &fillers {
            result = result.replace(filler, "");
        }
        Ok(result)
    }
}
