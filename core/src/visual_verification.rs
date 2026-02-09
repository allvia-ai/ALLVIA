use crate::llm_gateway::LLMClient;
use crate::visual_driver::VisualDriver;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct VisualVerifyRequest {
    pub prompts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VisualVerdict {
    pub prompt: String,
    pub ok: bool,
    pub response: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VisualVerifyResult {
    pub ok: bool,
    pub verdicts: Vec<VisualVerdict>,
}

pub async fn verify_screen(
    llm: &dyn LLMClient,
    req: VisualVerifyRequest,
) -> Result<VisualVerifyResult> {
    if req.prompts.is_empty() {
        return Ok(VisualVerifyResult {
            ok: true,
            verdicts: vec![],
        });
    }

    let (b64, _scale) = VisualDriver::capture_screen()?;
    let mut verdicts = Vec::new();

    for prompt in req.prompts {
        let full_prompt = format!(
            "Screen Verification Task.\nCondition to verify: '{}'.\nReply ONLY with 'YES' or 'NO'.",
            prompt
        );
        let mut response_text = None;
        let ok = match llm.analyze_screen(&full_prompt, &b64).await {
            Ok(resp) => {
                response_text = Some(resp.clone());
                resp.trim().to_uppercase().starts_with("YES")
            }
            Err(_) => false,
        };
        verdicts.push(VisualVerdict {
            prompt,
            ok,
            response: response_text,
        });
    }

    let ok = verdicts.iter().all(|v| v.ok);
    Ok(VisualVerifyResult { ok, verdicts })
}
