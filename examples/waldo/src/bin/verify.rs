// Copyright 2023 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
#![feature(type_ascription)]

use std::error::Error;

use clap::Parser;
// use waldo_methods::IMAGE_CROP_ID;
use waldo::Args;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let args = Args::parse();
    // verify_image(receipt);
    waldo::verify_image(&args)?;

    if args.no_display {
        println!(
            "IMPORTANT: Verify that the cutout in {} contains Waldo.",
            &args.waldo.display()
        );
    } else {
        // Display the image in the terminal for them to see whether it's Waldo.
        let viuer_config = viuer::Config {
            absolute_offset: false,
            ..Default::default()
        };
        viuer::print_from_file(&args.waldo, &viuer_config)?;
        println!("Prover knows where this cutout is in the given image.");
        println!("Do you recognize this Waldo?");
    }

    Ok(())
}
