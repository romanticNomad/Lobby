
---

# Lobby Core Architecture Overview

Lobby is an **EVM transaction signing and execution platform** built for determinism, auditability, and failure-tolerant operation.
It is structured as a set of **explicitly layered crates**, each with a narrow, well-defined role.

At a high level, Lobby is composed of:
* **crate**
    * **kernel** – shared foundations and authority contracts
    * **actors** – long-lived, state-owning decision makers
    * **constante** – deterministic, stateless computation
* **bin**
    * **main** – the server entry point and wiring layer

This separation is intentional and enforced throughout the codebase.

---

## Kernel: Foundational Types and Contracts

The **kernel** crate defines the *language and rules* of Lobby.

It contains:

* Core domain types (`ExecutionId`, `Intent`, identifiers, state enums)
* Traits that define **capabilities and authority boundaries**
* Invariants shared across all crates

The kernel:

* Has **no async runtime**
* Performs **no I/O**
* Depends on nothing above it

It exists to ensure that every part of Lobby agrees on *what things mean*, even as implementations evolve.

---

## Actors: Authoritative Runtime Components

**Actors** are **long-lived Tokio tasks** that **own mutable, order-sensitive state** and make **final decisions**.

Each actor:

* Is the **single writer** for its category of state
* Processes messages sequentially
* Persists authoritative facts to PostgreSQL
* Is resilient to crashes, retries, and duplication

### Actors in Lobby

#### RelayHost (Ingress Actor)

* Accepts external requests (EIP-1193, WalletConnect, relays, localhost)
* Normalizes them into `Intent`s
* Enforces ingress idempotency and rate limits
* Submits intents into the execution pipeline

Does **not** perform signing, nonce assignment, or broadcasting.

---

#### NonceManager (Ordering Actor)

* Owns the nonce timeline for `(chain_id, from)`
* Serializes nonce reservation, reuse, replacement, and resolution
* Enforces strict nonce ordering and idempotency

No other component may assign or reuse nonces.

---

#### Signer (Custody Actor)

* Decides whether a transaction may be signed
* Serializes signing per key
* Interfaces with local keys, HSMs, or AWS KMS
* Produces auditable signing artifacts

This is the **custody authority** of Lobby.

---

#### Broadcaster (Execution Actor)

* Submits signed transactions to the network
* Manages retries, gas bumping, and replacements
* Coordinates with the NonceManager for replacement flows

Owns execution-level economic decisions.

---

#### Validator (Truth Actor)

* Observes chain state
* Handles confirmations, reorgs, and finality thresholds
* Determines final execution outcomes

This actor defines **on-chain truth**.

---

## Constante: Deterministic Computation

The **constante** crate contains **pure or bounded, deterministic logic** that does **not** require its own runtime task and does **not** own state authority.

### Constants in Lobby

Constante includes:

* **Intent Normalization**

  * Converts external client requests into canonical `Intent`s

* **StateManager**

  * A stateless API for accessing the local database
  * Used by actors and the pipeline
  * Does **not** own or decide state

* **Canonicalizer**

  * RLP encoder/decoder
  * DER encoder/decoder
  * Canonical serialization for Lobby data structures

Constante logic:

* Is safe to run inline
* Is easy to test
* Contains no business authority

Rule of thumb:

> *If it is deterministic and does not own state over time, it belongs in constante.*

---

## Main: Server Entry Point

The **main** crate is the **runtime entry point** for Lobby.

It is responsible for:

* Bootstrapping the Tokio runtime
* Initializing PostgreSQL and Redis connections
* Spawning actors
* Wiring channels and queues between components
* Starting ingress transports and APIs

The `main.rs` file is where:

* All crates are composed
* Runtime topology is defined
* The Lobby server is brought to life

Importantly:

* `main` contains **no business logic**
* All authority lives in actors
* All semantics come from the kernel

---

## Execution Model Summary

1. External requests enter via **RelayHost**
2. Requests are normalized using **constante**
3. The pipeline emits commands
4. **Actors** make authoritative decisions
5. Facts are persisted to PostgreSQL
6. Redis coordinates execution and wakeups

Key rule:

> **Actors decide. Constante computes. Kernel defines. Main wires.**

---

## Design Principles

* Single-writer state ownership
* Explicit authority boundaries
* Determinism over convenience
* Idempotency everywhere
* Auditability by design

Lobby is built to scale from local signing to institutional custody without architectural rewrites.
