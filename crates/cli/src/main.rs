//! Headless tools for WarARSelet: demo matches, simulation, map previews
//! and benchmarks.

mod args;
mod map_view;
mod runner;

use args::Action;

fn main() {
    let options = match args::parse() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("warsel: {error}\n\n{}", args::usage());
            std::process::exit(2);
        }
    };

    let result = match options.action {
        Action::Help => {
            println!("{}", args::usage());
            Ok(())
        }
        Action::Demo | Action::Simulate => runner::simulate(&options),
        Action::Bench => runner::bench(&options),
        Action::Map => {
            map_view::print(&options);
            Ok(())
        }
    };

    if let Err(error) = result {
        eprintln!("warsel: {error}");
        std::process::exit(1);
    }
}
