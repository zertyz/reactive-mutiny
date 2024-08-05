# Objective

Since the performance is a key requirement of this crate, With these benchmarks, we aim to track the performance, over time, of some real-ish world usage scenarios -- so that we can be sure no performance regressions are introduced either by our sources, Rust versions, operating system, etc.

With this we also have a glimpse on how this crate behaves on different hardware.

# Guide

Here you will find the following directories:
  * `old`: legacy, less structured benchmarks
  * `<computer_name>`: the new, more rigirous benchmarks. Each directory will have a `machine.info` file describing the hardware.

The methodology for the new benchmarks are as follows:
  * Identification: inside each directory, the benchmarks will be named as: `<commit-hash>-<rust-version>-<kernel-version>.txt`
  * Compilation: `export RUSTFLAGS="-C target-cpu=native"; cargo test --release`
  * Execution: with a freshly booted machine, without any other running processes, the following is executed:
```
clear; rm /tmp/*.mmap; sudo sync; sleep 10; ./target/release/examples/all-channels
```
  * Summarization: For each measurement `all-channels` yield, the contents of the benchmark will have the individual best of 5 runs.
