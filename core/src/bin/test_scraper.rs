// use local_os_agent::dynamic_controller::DynamicController;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("🧪 Starting Visual Scraper Test...");

    // 1. Initialize
    let llm = std::sync::Arc::new(local_os_agent::llm_gateway::OpenAILLMClient::new()?);
    let planner = local_os_agent::controller::planner::Planner::new(llm, None);

    // 2. Goal: Read the screen
    // We expect the agent to capture, plan "read", and output text.
    // Since we can't easily assert the internal history state in this binary without modifying lib,
    // we will rely on stdout logs to verify "Read Info" appears.

    planner
        .run_goal(
            "Tell me what application is currently active/frontmost",
            None,
        )
        .await?;

    // We limit max steps to 2 to prevent endless loops if it fails
    // Ideally step 1: Read, Step 2: Done.
    planner.run_goal("Use the 'read' tool to describe the screen content in detail. Do NOT just say done. Read the screen first.", None).await?;

    println!("✅ Test Execution Complete. Check logs for 'Read Info' and 'Extracted'.");
    Ok(())
}
