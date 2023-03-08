# To run all tests (and @Ignored benchmarks) for all commits (remembering which commits were already executed)

```
export RUSTFLAGS="-C target-cpu=native -C target-feature=-avx"; mkdir -p /tmp/benchmarks; git pull; git log --pretty=oneline | tac | sed 's| .*||' >/tmp/benchmarks/git.hashes; touch /tmp/benchmarks/benchmarked.hashes; for git_hash in `comm -23 /tmp/benchmarks/git.hashes /tmp/benchmarks/benchmarked.hashes`; do git reset --hard $git_hash; name="`date +%s`.${git_hash}"; timeout 3600 cargo test --release -- --test-threads 1 -Z unstable-options --report-time --nocapture 2>&1 | tee /tmp/benchmarks/${name}.tests; timeout 3600 cargo test --release performance_measurements -- --test-threads 1 --nocapture 2>&1 | tee /tmp/benchmarks/${name}.performance; timeout 3600 cargo test --release -- --test-threads 1 --nocapture --ignored | tee /tmp/benchmarks/${name}.benchmarks; echo $git_hash >>/tmp/benchmarks/benchmarked.hashes; done
```
