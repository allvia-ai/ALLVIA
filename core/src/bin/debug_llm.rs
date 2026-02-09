use local_os_agent::llm_gateway::{LLMClient, OpenAILLMClient};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();

    let client = OpenAILLMClient::new()?;
    println!("🤖 Testing LLM with input '야'...");

    let ui_dummy = json!({
        "type": "window",
        "children": []
    });

    // We need to bypass the method to see raw response if the method swallows it.
    // But let's try calling the method first. If it errors, we see the error.
    // Actually, to see the RAW response, I should probably inspect the code or modify the client temporarily.
    // But simplest first: run it and see the panic/error message detail.
    // Wait, the user sees "No content in LLM response".

    // Let's implement a direct call similar to `plan_next_step` but printing body.
    match client.plan_next_step("야", &ui_dummy, &[]).await {
        Ok(res) => println!("✅ Success: {:?}", res),
        Err(e) => println!("❌ Error: {:?}", e),
    }

    Ok(())
}
