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
#![feature(error_in_core)]

use core::error::Error;

use clap::Parser;
use waldo::verify::Args;

fn main() -> Result<(), Box<dyn Error>> {
    #[cfg(not(feature = "minimal"))]
    env_logger::init();

    #[cfg(not(feature = "minimal"))]
    let args = Args::parse();
    #[cfg(not(feature = "minimal"))]
    waldo::verify::verify_image(&args)?;

    Ok(())
}
