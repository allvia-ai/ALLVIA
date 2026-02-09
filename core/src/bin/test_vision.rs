use local_os_agent::visual_driver::{SmartStep, UiAction, VisualDriver};
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("🧪 Starting Vision Test...");

    // Initialize LLM (requires OPENAI_API_KEY in .env)
    let llm = std::sync::Arc::new(
        local_os_agent::llm_gateway::OpenAILLMClient::new().expect("Failed to init LLM"),
    );

    // Create a Vision Driver
    let mut driver = VisualDriver::new();

    // Add a vision step (User should open a window with a 'File' menu or similar first)
    println!("Please ensure the 'Apple' logo () is visible in the top left corner.");
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    driver.add_step(SmartStep::new(
        UiAction::ClickVisual("Apple logo in the menu bar".to_string()),
        "Clicking Apple Logo via Vision",
    ));

    // Execute
    match driver.execute(Some(llm.as_ref())).await {
        Ok(_) => println!("✅ Test Passed!"),
        Err(e) => println!("❌ Test Failed: {}", e),
    }

    Ok(())
}
