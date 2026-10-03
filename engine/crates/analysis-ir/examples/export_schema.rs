//! Prints the JSON Schema of `ParsedUnit` (documentation only; the IR is not a cross-language
//! contract).
//!
//!   cargo run -q -p analysis-ir --example export_schema > docs/graph-schema/ir.schema.json

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let schema = schemars::schema_for!(analysis_ir::ParsedUnit);
    println!("{}", serde_json::to_string_pretty(&schema)?);
    Ok(())
}
