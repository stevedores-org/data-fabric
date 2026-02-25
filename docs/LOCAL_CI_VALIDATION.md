# Local CI Validation

This repo uses [local-ci](https://github.com/stevedores-org/local-ci) to run the same checks locally that CI runs on GitHub Actions. The configuration lives in `.local-ci.toml`.

## Quick Start

```bash
# Install local-ci (requires Go)
go install github.com/stevedores-org/local-ci@latest

# Run all stages
local-ci

# Run with JSON output
local-ci --json
```

## Stages

| Stage   | Command                                              | Enabled |
|---------|------------------------------------------------------|---------|
| fmt     | `cargo fmt --all -- --check`                         | Yes     |
| clippy  | `cargo clippy --all-targets -- -D warnings`          | Yes     |
| check   | `cargo check --target wasm32-unknown-unknown`        | Yes     |
| test    | `cargo test`                                         | Yes     |
| deny    | `cargo deny check`                                   | No      |
| audit   | `cargo audit`                                        | No      |

## Coverage

Coverage is separate from local-ci stages. Use the Makefile targets:

```bash
# Print coverage summary (fails if below 75%)
make coverage

# Generate and open an HTML report
make coverage-html
```

Requires `cargo-llvm-cov`:
```bash
cargo install cargo-llvm-cov
```

## TDD Recipes

### Recipe: Add a Test to `data-fabric-worker`

This is a single-crate repo. The TDD loop is straightforward.

**1. Write a failing test**

```rust
// src/lib.rs or src/<module>.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn my_new_behavior() {
        // Arrange
        let input = ...;
        // Act
        let result = my_function(input);
        // Assert
        assert_eq!(result, expected);
    }
}
```

**2. Run local-ci to confirm the test fails**

```bash
local-ci
# Expected: test stage fails with your new test
```

**3. Implement the feature**

Write the minimal code to make the test pass.

**4. Run local-ci again**

```bash
local-ci
# Expected: all stages pass
```

**5. Check coverage**

```bash
make coverage
# Expected: 75%+ line coverage maintained
```

**6. Commit and push**

```bash
git add -A && git commit -m "feat: describe what you added"
git push -u origin $(git branch --show-current)
```

### Recipe: Fix a Bug with TDD

**1. Write a test that reproduces the bug**

```rust
#[test]
fn bug_description_reproduces() {
    // Set up the conditions that trigger the bug
    let result = buggy_function(trigger_input);
    assert_eq!(result, correct_output); // This should FAIL
}
```

**2. Confirm the test fails:** `local-ci`

**3. Fix the bug** — minimal change to make the test pass.

**4. Confirm all tests pass:** `local-ci`

**5. Verify coverage:** `make coverage`
