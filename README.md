# TriCTI

TriCTI is a high-performance (theoretically, it's too early to properly test), concurrent, data-oriented programming language that unifies **ECS architectures, relational databases, array-oriented computation, and reactive programming** in a Rust-inspired, statically analyzable framework. It emphasizes **implicit optimization** via static concurrency analysis, SIMD/vectorized operations, and optional GPGPU acceleration.

---
## Quick Start

Common development commands are available via `make`.

### Build

```bash
make build
```

Compiles the project using Cargo.

### Run

```bash
make run
```

Runs the executable, automatically loading `.env` if present.

### Test

```bash
make test
```

Runs both Cargo and TriCTI tests.

To pass arguments, use `--` to separate them from `make`:

- Run only TriCTI tests:

  ```bash
  make test -- -t
  ```

- Run only Cargo tests:

  ```bash
  make test -- -c
  ```

- Run specific TriCTI tests:

  ```bash
  make test -- -t foo/bar.tri
  ```

  (Resolves to `tests/tests/foo/bar.tri`)

- Run specific Cargo tests:

  ```bash
  make test -- -c --test my_module
  ```

- Run both with arguments:

  ```bash
  make test -- -t foo/bar.tri -c --test my_module
  ```

### Format and Lint

```bash
make fmt
```

Runs code formatting and lint checks.

### Clean

```bash
make clean
```

Removes build artifacts.

### Setup

```bash
make setup
```

Marks all scripts in `scripts/` as executable and prepares `.env`.

---

After cloning, run `make setup` once to initialize the project scripts.

## Design Motivation

TriCTI was created to generalize multiple paradigms into a coherent language capable of:

- Expressing **per-trigger DAGs** for deterministic reactive execution.
- Supporting **array-language semantics** for vectorized operations and hardware acceleration.
- Integrating relational abstractions for safe, efficient state management.

While Rust macros could approximate much of this, they lack shared global state, making **static DAG analysis, query inspection, and safe concurrency heuristics** infeasible. GPGPU execution would require explicit management via libraries like **rust-gpu**, and dynamic scheduling would prevent full compile-time optimization. Implementing TriCTI as an **LLVM frontend** enables native expression of vectorized, GPU-aware operations and static scheduling analysis.

The design was inspired by practical experience with **Gnu Octave** (array and SIMD semantics), **Axum** (reactive programming), **SQL** via rusqlite (relational state management), and [a custom Bevy-inspired ECS](https://github.com/slangerosuna/klaus_of_death_again). Another key motivation was a personal interest in **learning compiler construction**.

---

## Architecture Overview

TriCTI organizes computation around **per-trigger DAGs** (cylic graphs may eventually be allowed to enable certain behavior that may benefit). Multiple DAGs can execute concurrently, with the runtime enforcing **safe parallelism** via static interference detection. Users declare mutation behavior—element-wise, vector-wide, or MVCC-friendly—guiding automatic concurrency semantics.  

**Signals** coordinate interactions, supporting static polymorphism over topologies (MPSC, SPMC, MPMC) and buffering (FIFO, overwrite, fail-on-unconsumed). Array-language semantics allow vectorized operations, and critical systems can be **compiled to OpenCL kernels via LLVM** for predictable GPGPU execution. **PostgreSQL bindings** provide an efficient relational backend, minimizing the need to reimplement indexing or query execution.

TriCTI combines **imperative logic** with declarative scheduling, offering a framework for deterministic, composable, and high-performance distributed systems.

---

## Current State

The project is currently **bootstrapping the Rust-based frontend**, implementing parsing and compiler infrastructure. Future goals include completing the DAG scheduler, PostgreSQL integration, GPGPU code generation, and system/interaction composition tooling.
