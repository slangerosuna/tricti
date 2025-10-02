# TriCTI

TriCTI (Codd-Church-Curry-Tarski-Iverson)is a high-performance (theoretically, it's too early to properly test), concurrent, data-oriented programming language that unifies **ECS architectures, relational databases, array-oriented computation, and reactive programming** in a Rust-inspired, statically analyzable framework. It emphasizes **implicit optimization** via static concurrency analysis, SIMD/vectorized operations, and optional GPGPU acceleration.

---

## At a Glance

```tri
use prelude
use rt

# -------------------------------
# Generic and utility functions
# -------------------------------
generic_identity<T> :: (value: T) -> T => value

add :: (x: i32, y: i32) => x + y
# !i32 is syntax sugar for Result<i32, *dyn Error>
div :: (x: i32, y: i32) -> !i32 => {
    if y == 0 {
        # `!expr` wraps the type of expr (which has to impl `Error`) in an owned pointer to a trait object (*dyn Error)
        # this would be equivalent to `return Err(Box::new("Cannot divide by zero"))` in Rust
        ret !"Cannot divide by zero"
    }
    # Wrapping in Ok or Some is implicit unless you use the `some` or `ok` keywords explicitly
    x / y
}

# -------------------------------
# Compile-time evaluation
# -------------------------------
@const
PI_APPROX :: f32 := 3.14159

# because of @par_const this evaluates to a section of .data in the binary
# that is allocated to the heap at runtime
# [i32] here is just syntax sugar for std::core::collections::Vec<i32>
@par_const
make_array :: (n: i32) -> [i32] => {
    arr := [0; n]
    for i in 0:n { arr[i] = i * i }
    arr
}

# -------------------------------
# Memoized functions
# -------------------------------
@memoize
fib :: (n: i32) -> i32 => match n {
    0 => 0,
    1 => 1,
    n => fib(n - 1) + fib(n - 2),
}

default_args :: (a: i32 = 17) => {}

# -------------------------------
# Structs, enums, references
# -------------------------------
MyStruct :: {
    # There are four types of pointers in Tricti that interact different ways with the borrow checker:
    # 1. Owned pointers: `*T` - own the data they point to, frees the data when they go out of scope
    # 2. Shared pointers: `&T` - shared immutable references to data owned by someone else
    # 3. Mutable references: `&mut T` - exclusive mutable references to data owned by someone else
    # 4. Raw pointers: `*raw T` - these are unsafe pointers that do not have any ownership semantics
    #    and can be null or dangling. They are used for FFI and low-level programming. Important to
    #    note that dereferencing is unsafe and you must free the memory manually.

    # All 4 of these pointer types, by default, are non-nullable(except *raw T, but you are supposed
    # to use ?*raw T if it would actually wind up null)
    # To make any of these pointer types nullable, just prefix with a `?` to make it an optional
    # e.g. `?*T`, `?&T`, `?&mut T`, `?*raw T`

    a: ?i32,
    b: ?&i32,
    c: &mut i32,
    d: ?*MyStruct,
}

impl MyStruct {
    foo :: (&self) -> ?i32 => self.a? + *self.b?
}

MyEnum :: enum {
    A: MyStruct,
    B: { a: ?i32, b: ?i32 },
}

Image :: {}
Gui :: @resource {}

# -------------------------------
# Database and tables
# -------------------------------
Apps :: table {
    @primary id: u64,
    title: String,
    image: Image,
    display: bool = false,
}

Database :: db { Apps: Apps, Gui }

# -------------------------------
# Signals
# -------------------------------
ExampleSignal :: Signal<Mpsc, Buffer, String>
RedrawRequested :: Signal<Spmc, Overwrite, none>

# -------------------------------
# Systems
# -------------------------------
display_apps :: sys (
   query: select (image: &Image, title: &String) from Apps where display == true,
   gui: res &mut Gui,
   @sys_input input_size: f32
) -> !none => {
  query.for_each((row: (image: &Image, title: &string)) => {
      gui.display(image, title)
  })
}

# Example function offloaded to GPU for vectorized computation
# @gpu hands it off to an OpenCL kernel explicitly, but in future
# versions the compiler will do this implicitly depending on heuristics
@gpu
vector_add :: (arr_a: [f32], arr_b: [f32]) -> [f32] => arr_a .+ arr_b

ExampleSignal :: Signal<Mpsc, Buffer, String>

emitter_sys :: sys () => { emit(ExampleSignal, "Frame Finalized") }

@trigger ExampleSignal
receiver_sys :: sys (@trigger_recv msg: String) => { println("Received: {}", msg) }

# -------------------------------
# Interaction composition
# -------------------------------
Table :: table {
    @primary id: u64,
    foo: Foo,
    bar: Bar,
}

Foo :: {}
Bar :: {}
Quux :: {}

# here the expression `Quux { }` is wrapped in Some explicitly with the `some` keyword
# however, this is not necessary in the return statement of functions that return Option types
# e.g. `Quux { }` would be equivalent
#
# wrapping in Some explicitly is really just there if the end user prefers it
init_foobar :: sys (query: select (Bar) from Table) -> ?Quux { some Quux { } }
foo :: sys (query: select (Foo) from Table, @sys_input quux: Quux) -> Baz { Baz { } }
bar :: sys (query: select (Bar) from Table, @sys_input baz: Baz) -> ?Quux { none }

foobar :: compose {
    # runs init_foobar, if it returns a Quux, passes it to foo
    # takes the Baz result from foo and passes it to bar
    # if bar returns a Quux, passes it back to foo, creating a feedback loop
    # that terminates when bar returns none
    init_foobar ?-> (a: foo) -> (b: bar),
    (b: bar) ?-> (a: foo),
}

# -------------------------------
# Triggers
# -------------------------------
@trigger(Init)
setup_signals :: () => {
    println("Initializing signals and DAGs")
}

RedrawRequested :: Signal<Spmc, Overwrite, none>
@trigger(RedrawRequested, Database)
redraw :: compose {
    display_apps -> emitter_sys,

    # fan out, merge
    # foo -> (bar, baz) -> quux,
    # static fanout and merge operates via tuple composition/decomposition
    # although merge here can also work by copying the value if it is a Copy type and not a tuple
    # more complicated DAGs are possible too
    # (a: foo) -> ((b: bar), (c: baz)) -> quux,
    # (b: bar) -> (c: baz)
    # as mentioned above, feedback loops are possible too
    # (c: baz) ?-> (b: bar)
    # and there is also dynamic fanout and merge via decomposing and composing vectors
    # foo -> [bar], # runs bar for each element of the vector returned by foo
    # [bar] -> quux, # merges the results of all bar instances into a vector and passes to quux
}

# -------------------------------
# Vector and matrix operations
# -------------------------------
example_arrays :: () => {
    arr := [1, 2, 3]
    println(arr.map(add.bind(y: 1)))
    println(arr.filter((x) => x % 2 == 0))

    matrix := [1 0 0; 0 1 0; 0 0 1]
    matrix *= [0 1 0; 1 0 1; 0 1 0]
    println(matrix)
}

# -------------------------------
# GPGPU example usage
# -------------------------------
gpu_example :: () => {
    a := [1.0, 2.0, 3.0]
    b := [4.0, 5.0, 6.0]
    c := vector_add(a, b) # executed on GPU via OpenCL kernel
    println(c)
}
```

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

- Set a custom time out for the run of `cargo test` to handle hanging calls

  ```bash
  make test -- -to 10s
  ```

  (Or to bypass the timeout)

  ```bash
  make test -- -to none
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
