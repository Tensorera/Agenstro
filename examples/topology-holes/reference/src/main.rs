use std::env;
use std::error::Error;
use std::fs;
use std::process::ExitCode;

use topology_holes::{Grid, Topology, analyze};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("topology-holes: {message}");
            ExitCode::FAILURE
        }
    }
}
fn run() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args_os();
    let _program = arguments.next();
    let path = arguments
        .next()
        .ok_or("usage: topology-holes <grid-file>")?;
    if arguments.next().is_some() {
        return Err("usage: topology-holes <grid-file>".into());
    }

    let source = fs::read_to_string(path)?;
    let result = analyze(&Grid::parse(&source)?);
    println!("{}", render_json(&result));
    Ok(())
}

fn render_json(result: &Topology) -> String {
    let hole_areas = result
        .hole_areas
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        concat!(
            "{{\"width\":{},\"height\":{},",
            "\"solidComponents\":{},\"holes\":{},",
            "\"eulerCharacteristic\":{},\"solidCells\":{},",
            "\"backgroundCells\":{},\"holeAreas\":[{}]}}"
        ),
        result.width,
        result.height,
        result.solid_components,
        result.holes,
        result.euler_characteristic,
        result.solid_cells,
        result.background_cells,
        hole_areas
    )
}
