use property_facilitator::{run_pack, Pack};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_pack(Pack::Hotels).await?;
    Ok(())
}
