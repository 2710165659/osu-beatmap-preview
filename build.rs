use vergen::BuildBuilder;
use vergen::Emitter;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let build = BuildBuilder::default().build_timestamp(true).build()?;
    Emitter::default().add_instructions(&build)?.emit()?;
    Ok(())
}
