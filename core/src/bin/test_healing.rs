use local_os_agent::visual_driver::{SmartStep, UiAction, VisualDriver};
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("🧪 Starting Self-Healing Test (Robustness Check)...");

    let llm = std::sync::Arc::new(local_os_agent::llm_gateway::OpenAILLMClient::new()?);
    let mut driver = VisualDriver::new();

    // Add a step to click a non-existent element
    driver.add_step(SmartStep::new(
        UiAction::ClickVisual("NonExistentGhostButton_XYZ".to_string()),
        "Clicking Ghost Button",
    ));

    println!("👻 Expecting retries... (This should take ~6-8 seconds then fail)");

    // Execute
    if let Err(e) = driver.execute(Some(llm.as_ref())).await {
        println!("✅ Test Passed! Caught expected error after retries: {}", e);
    } else {
        println!("❌ Test Failed! Logic claimed success on non-existent button.");
    }

    Ok(())
}
