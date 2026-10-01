//! Replaces a SciWhisper installation once the application holding it open
//! has exited. Started by the application; not meant to be run by hand.

use std::process::ExitCode;

use sciwhisper_update::helper;

fn main() -> ExitCode {
    let args = match helper::parse_args(std::env::args_os().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    match helper::run(&args, &mut std::thread::sleep, &mut helper::spawn_detached) {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
