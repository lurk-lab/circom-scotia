// Copyright (c) 2022 Nalin
// Copyright (c) Lurk Lab
// SPDX-License-Identifier: MIT
//! # Circom Scotia
//!
//! `circom_scotia` is a middleware library that facilitates the use of [Circom](https://github.com/iden3/circom)
//! circuits with the [Bellpepper](https://github.com/lurk-lab/bellpepper) proving system. It allows for the
//! compilation of Circom circuits and the generation and synthesis of witnesses using Bellpepper.
//!
//! This library is based on [Nova-Scotia](https://github.com/nalinbhardwaj/Nova-Scotia) and Arkworks'
//! [Circom-Compat](https://github.com/arkworks-rs/circom-compat), adapted to work with the Bellpepper ecosystem.
//! It supports the Vesta curve and handles R1CS constraints and witness generation in a manner compatible
//! with Circom's output format.
//!
//! ## Features
//!
//! - Loading and parsing of R1CS constraints generated by the Circom compiler.
//! - Generation of witnesses from WASM binaries produced by Circom.
//! - Integration with Bellpepper's constraint system for zk-SNARK proofs.
//!
//! ## Usage
//!
//! The primary entry points of this library are functions for loading R1CS files, generating witnesses
//! from WASM, and synthesizing constraints within a Bellpepper environment.
//!
//! ## Contributions and Credits
//!
//! Contributions are welcome.
//! Credits to the [Circom language](https://github.com/iden3/circom) team, [Nova-Scotia](https://github.com/nalinbhardwaj/Nova-Scotia),
//! and [ark-circom](https://github.com/gakonst/ark-circom) for their foundational work that this library builds upon.

use std::{
    env::current_dir,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::error::WitnessError::{
    self, FailedExecutionError, FileSystemError, LoadWitnessError, MutexError,
    WitnessCalculationError,
};
use crate::r1cs::CircomInput;
use anyhow::Result;
use bellpepper_core::{num::AllocatedNum, ConstraintSystem, LinearCombination, SynthesisError};
use ff::PrimeField;
use r1cs::{CircomConfig, R1CS};

use crate::reader::load_witness_from_file;

pub mod error;
pub mod r1cs;
pub mod reader;
pub mod witness;

/// Generates a witness file from a given WebAssembly (WASM) binary using a JSON input.
///
/// This function takes the path to the directory containing the WASM binary and JavaScript
/// (JS) files, a JSON string as input to the WASM, and a path to output the witness file.
/// It returns a vector of field elements if successful or an error otherwise.
///
/// To generate the necessary files, please refer to https://docs.circom.io/getting-started/compiling-circuits/.
///
/// # Arguments
///
/// * `witness_dir` - A [`PathBuf`] representing the directory containing the WASM and JS files.
/// * `witness_input_json` - A [`String`] containing the JSON input for the WASM.
/// * `witness_output` - A reference to the path where the output witness file will be stored.
///
/// # Errors
///
/// Returns an error if any file operations fail, or if the WASM execution fails.
///
/// # Examples
///
/// ```no_run
/// # use std::path::PathBuf;
/// # use circom_scotia::generate_witness_from_wasm;
///
/// let witness_dir = PathBuf::from("./path/to/witness/dir");
/// let input_json = "{\"input\": \"value\"}".to_string();
/// let witness_output = PathBuf::from("output.wtns");
/// let result = generate_witness_from_wasm(witness_dir, input_json, &witness_output);
/// ```
pub fn generate_witness_from_wasm<F: PrimeField>(
    witness_dir: PathBuf,
    witness_input_json: String,
    witness_output: impl AsRef<Path>,
) -> Result<Vec<F>, WitnessError> {
    // Create the input.json file.
    let root = current_dir().map_err(|err| FileSystemError(err.to_string()))?;
    let witness_generator_input = root.join("circom_input.json");
    fs::write(&witness_generator_input, witness_input_json)
        .map_err(|err| FileSystemError(err.to_string()))?;

    // Prepare and execute the node cmd to generate our witness file.
    let mut witness_js = witness_dir.clone();
    witness_js.push("generate_witness.js");
    let mut witness_wasm = witness_dir.clone();
    witness_wasm.push("main.wasm");

    let output = Command::new("node")
        .arg(&witness_js)
        .arg(&witness_wasm)
        .arg(&witness_generator_input)
        .arg(witness_output.as_ref())
        .output()
        .map_err(|err| FailedExecutionError(err.to_string()))?;

    // Print output of the node cmd.
    if !output.stdout.is_empty() || !output.stderr.is_empty() {
        println!("stdout: {}", std::str::from_utf8(&output.stdout).unwrap());
        println!("stderr: {}", std::str::from_utf8(&output.stderr).unwrap());
    }

    // Tries to remove input file. Warns if it cannot be done.
    let res = fs::remove_file(witness_generator_input);
    if res.is_err() {
        println!("warning: could not cleanup temporary files")
    }

    // Reads the witness from the generated file.
    load_witness_from_file(witness_output).map_err(|err| LoadWitnessError(err.to_string()))
}

/// Calculates a witness for a given R1CS configuration and a set of circuit inputs.
///
/// The function locks the global witness calculation instance and then calculates
/// the witness based on the inputs provided. It performs a sanity check if required.
///
/// # Arguments
///
/// * `cfg` - A reference to the [`CircomConfig`] containing R1CS configuration.
/// * `input` - A vector of [`CircomInput`], representing the inputs to the circuit.
/// * `sanity_check` - A boolean indicating whether a sanity check should be performed.
///
/// # Errors
///
/// Returns an error if witness calculation fails or if the mutex lock cannot be acquired.
///
/// # Examples
///
/// ```no_run
/// # use std::path::{Path, PathBuf};
/// # use circom_scotia::{calculate_witness, CircomConfig, CircomInput};
/// # use ff::Field;
/// # use pasta_curves::vesta::Base as Fr;
///
/// let wtns = PathBuf::from("circuit.wtns");
/// let r1cs = PathBuf::from("circuit.r1cs");
///
/// let cfg = CircomConfig::new(wtns, r1cs);
/// let inputs = vec![CircomInput::new(String::from("input_name"), vec![Fr::ZERO])];
/// let result = calculate_witness(&cfg, inputs, true);
/// ```
pub fn calculate_witness<F: PrimeField>(
    cfg: &CircomConfig<F>,
    input: Vec<CircomInput<F>>,
    sanity_check: bool,
) -> Result<Vec<F>, WitnessError> {
    let mut lock = cfg.wtns.lock().map_err(|_| MutexError)?;
    let witness_calculator = &mut *lock;
    witness_calculator
        .calculate_witness(input, sanity_check)
        .map_err(|err| WitnessCalculationError(err.to_string()))
}

/// Synthesizes the constraint system based on the R1CS and the witness data.
///
/// This function updates the provided constraint system based on the R1CS constraints
/// and the witness data. It returns the public outputs of the circuit as [`AllocatedNum`].
///
/// # Arguments
///
/// * `cs` - A mutable reference to the constraint system.
/// * `r1cs` - The [`R1CS`] data structure.
/// * `witness` - An optional vector of field elements representing the witness data.
///
/// # Errors
///
/// Returns a [`SynthesisError`] if constraint synthesis fails.
///
/// # Notes
///
/// Reference work is Nota-Scotia: https://github.com/nalinbhardwaj/Nova-Scotia
pub fn synthesize<F: PrimeField, CS: ConstraintSystem<F>>(
    cs: &mut CS,
    r1cs: R1CS<F>,
    witness: Option<Vec<F>>,
) -> Result<Vec<AllocatedNum<F>>, SynthesisError> {
    let witness = &witness;
    let mut vars: Vec<AllocatedNum<F>> = vec![];

    // Retrieve all our public signals (inputs and outputs).
    for i in 1..r1cs.num_inputs {
        let f: F = {
            match witness {
                None => F::ONE,
                Some(w) => w[i],
            }
        };
        let v = AllocatedNum::alloc(cs.namespace(|| format!("public_{}", i)), || Ok(f))?;

        vars.push(v);
    }

    // Retrieve all private traces.
    for i in 0..r1cs.num_aux {
        let f: F = {
            match witness {
                None => F::ONE,
                Some(w) => w[i + r1cs.num_inputs],
            }
        };

        let v = AllocatedNum::alloc(cs.namespace(|| format!("aux_{}", i)), || Ok(f))?;
        vars.push(v);
    }

    // Public output to return.
    let output = match r1cs.num_pub_out {
        0 => vec![],
        1 => vec![vars[0].clone()],
        _ => vars[0..r1cs.num_pub_out - 1usize].to_vec(),
    };

    // Create closure responsible to create the linear combination data.
    let make_lc = |lc_data: Vec<(usize, F)>| {
        let res = lc_data.iter().fold(
            LinearCombination::<F>::zero(),
            |lc: LinearCombination<F>, (index, coeff)| {
                lc + if *index > 0 {
                    (*coeff, vars[*index - 1].get_variable())
                } else {
                    (*coeff, CS::one())
                }
            },
        );
        res
    };

    for (i, constraint) in r1cs.constraints.into_iter().enumerate() {
        cs.enforce(
            || format!("constraint {}", i),
            |_| make_lc(constraint.0),
            |_| make_lc(constraint.1),
            |_| make_lc(constraint.2),
        );
    }

    Ok(output)
}
