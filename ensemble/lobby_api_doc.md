# Lobby Client API Documentation

> **Lobby** is a high-performance, low-latency blockchain transaction signing service designed for developers who need reliable, concurrent transaction processing at scale.

> ⚠️ **Prototype Notice:** Lobby is currently in active development. APIs, features, and behaviors described in this document may change in future releases. Please refer to the [GitHub repository](https://github.com/romanticNomad/Lobby) for the latest updates.

---

## Table of Contents

1. [Introduction](#1-introduction)
2. [System Architecture Overview](#2-system-architecture-overview)
3. [Core Concepts](#3-core-concepts)
4. [Authentication & Authorization](#4-authentication--authorization)
5. [API Endpoints Reference](#5-api-endpoints-reference)
6. [Transaction Lifecycle & Status Tracking](#6-transaction-lifecycle--status-tracking)
7. [Error Handling](#7-error-handling)
8. [Client Implementation Examples](#8-client-implementation-examples)
9. [Production Considerations](#9-production-considerations)
10. [Appendices](#10-appendices)

---

## 1. Introduction

### What is Lobby?

Lobby is a custodial Ethereum transaction signing service built for **high-throughput, low-latency** transaction processing. It abstracts away the complexity of:

- **Nonce management** — Sequential nonce assignment with automatic gap recovery
- **Transaction signing** — Secure key custody with EIP-1559 signature generation
- **Broadcast orchestration** — Parallel RPC submission with retry logic
- **Confirmation tracking** — Automated on-chain validation with reorg protection

### Key Features

- **Sub-second internal processing** — Nonce assignment and signing typically complete in < 1 second
- **Concurrent pipeline architecture** — Process thousands of transactions simultaneously via actor-based sharding
- **Automatic recovery** — Built-in retry logic for transient failures, nonce mismatch detection, and stale nonce cleanup
- **Multi-chain support** — Single API for Ethereum mainnet, Polygon, Arbitrum, and testnets
- **Real-time status tracking** — Poll transaction progress from submission to on-chain confirmation

### Use Cases

- **DApp backends** — Offload transaction signing and nonce management from your application
- **MEV bots** — Low-latency submission for time-sensitive arbitrage and liquidation transactions
- **Batch operations** — Process large volumes of transactions (airdrops, payroll, etc.) with automatic nonce sequencing
- **Multi-chain services** — Unified API for cross-chain transaction submission

### Design Philosophy

Lobby is designed around three core principles:

1. **Reliability over speed** — Every transaction is tracked end-to-end with automatic failure recovery
2. **Transparency** — Detailed status updates at every pipeline stage
3. **Simplicity** — Clean REST API with standard EIP-1193 request format

---

## 2. System Architecture Overview

### Pipeline Stages

Every transaction submitted to Lobby flows through a five-stage pipeline:

```
┌─────────────┐
│   Client    │
│   Submit    │
└──────┬──────┘
       │
       ▼
┌─────────────────────────────────────────────────────────────┐
│                    LOBBY PIPELINE                           │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  1. RelayHost        → Validate & persist transaction       │
│  2. Nonce Reserve    → Assign sequential nonce              │
│  3. Sign             → Generate EIP-1559 signature          │
│  4. Broadcast        → Submit to blockchain RPC             │
│  5. Validator        → Confirm on-chain inclusion           │
│                                                             │
└──────┬──────────────────────────────────────────────────────┘
       │
       ▼
┌─────────────┐
│  Confirmed  │
│  or Failed  │
└─────────────┘
```

### Actor-Based Sharding

Lobby achieves high concurrency through **sharded actor pools**:

| Actor Pool | Shard Key | Concurrency Benefit |
|---|---|---|
| **Nonce** | `from_address` | Sequential nonce assignment per address (no races) |
| **Sign** | `execution_id` | Parallel signing across independent transactions |
| **Broadcast** | `chain_id` | Per-chain RPC connection pooling |

This architecture allows Lobby to process **thousands of concurrent transactions** while maintaining strict ordering guarantees where needed (nonce assignment per address).

### Core Components

**Cortex (Orchestrator)**
- Coordinates the five-stage pipeline
- Manages backpressure via semaphore-gated concurrency limits
- Implements retry logic with exponential backoff

**StatusRegistry**
- In-memory transaction state tracking (DashMap + Redis persistence in production)
- Real-time status updates accessible via polling API
- Automatic cleanup for completed transactions

**Background Bots**
- **Sweeper Bot** — Releases expired nonce reservations (prevents nonce gaps)
- **Scanner Bot** — Polls RPC for timed-out transactions (recovers from validator failures)

---

## 3. Core Concepts

### 3.1 Execution ID

Every transaction submitted to Lobby is assigned a unique **execution ID** (UUID v4). This identifier:

- Tracks the transaction through all pipeline stages
- Enables status polling via the `/status/{execution_id}` endpoint
- Provides idempotency for retry scenarios

**Format:** `550e8400-e29b-41d4-a716-446655440000`

**Important:** The execution ID is **not** the on-chain transaction hash. The `tx_hash` becomes available only after the **Broadcast** stage completes.

---

### 3.2 Nonce Management

Lobby fully manages nonce assignment and lifecycle. Clients **never** provide a nonce field.

**Nonce States:**

| State | Description |
|---|---|
| `reserved` | Nonce assigned to a transaction, locked for 5 minutes |
| `finalized` | Transaction confirmed on-chain, nonce consumed |
| `released` | Pipeline failed, nonce returned to pool for reuse |
| `consumed` | Validator timed out, nonce marked consumed to prevent gaps |

**Nonce Mismatch Recovery:**

If the on-chain nonce differs from Lobby's database (e.g., external wallet sent a transaction), Lobby automatically:
1. Detects the mismatch during broadcast (`NonceTooLow` error)
2. Releases the incorrect nonce
3. Syncs with the RPC node to fetch the correct nonce
4. Re-signs the transaction with the corrected nonce
5. Retries broadcast once

This recovery happens **transparently** — clients see the status transition but no manual intervention is required.

---

### 3.3 Transaction Signing

Lobby is a **custodial signing service**. Private keys are stored server-side in a policy engine (currently `JsonPolicyEngine` for development, production deployments should use HSM/KMS).

**Signing Flow:**
1. Client submits unsigned EIP-1559 transaction
2. Lobby's Sign actor retrieves the private key for the authenticated `from_address`
3. Transaction is signed using ECDSA (secp256k1) with Keccak-256 hashing
4. Signature is RLP-encoded per EIP-1559 specification

**Signature Format:**
- `yParity` ∈ {0, 1} (EIP-1559 recovery ID)
- `r`, `s` — Canonicalized to **low-S** form (EIP-2)
- Leading zeros stripped for minimal RLP encoding

**Security Note:** In production, private keys should be managed via hardware security modules (HSM) or cloud key management services (AWS KMS, Google Cloud KMS). The current `JsonPolicyEngine` is **for development only**.

---

### 3.4 Broadcast & Validation

**Broadcast Actor:**
- Submits signed transactions to blockchain RPC nodes
- Sharded by `chain_id` for per-chain connection pooling
- Automatic retry on transient RPC failures
- Special handling for `NonceTooLow` (triggers nonce sync recovery)

**Validator Actor:**
- Polls RPC for transaction receipt (30-second intervals, 5-minute timeout)
- Confirms ≥3 block confirmations before marking `confirmed`
- Detects reverted transactions (status=0 in receipt)
- Handles validator timeout gracefully (Scanner Bot continues polling)

**RPC Provider Configuration:**

Lobby requires RPC endpoints to be configured via environment variables:

```bash
RPC_ENDPOINT_1=https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY
RPC_ENDPOINT_137=https://polygon-mainnet.g.alchemy.com/v2/YOUR_KEY
```

If no RPC provider is configured for a chain, broadcast will immediately fail with `MissingProvider` error.

---

### 3.5 Retry & Backoff Strategy

Lobby uses **full-jitter exponential backoff** for all retryable operations:

```
window = min(max_delay, base_delay × 2^attempt)
delay  = random(0, window)
```

**Default Configuration:**
- `max_attempts`: 2 (3 total tries including initial attempt)
- `base_delay`: 100ms
- `max_delay`: 200ms

**Retry Decision Logic:**

Operations return `RetryDecision<E>` to signal whether an error is retryable:

| Decision | Behavior |
|---|---|
| `Retry(e)` | Retry with exponential backoff |
| `FailImmediately(e)` | Abort retry loop, return error |

**Non-Retryable Errors:**
- `BroadcastError::MissingProvider` — No RPC configured for chain
- `BroadcastError::NonceTooLow` — Triggers nonce sync recovery (not retried as-is)
- `RelayHostError::ValidationFailed` — Client-side input error

---

## 4. Authentication & Authorization

### 4.1 API Key Format

Lobby uses **Bearer token authentication** with structured API keys:

```
lobby_live_<random_string>:<client_id>:<from_address>
```

**Example:**
```
lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92
```

**Components:**
| Part | Type | Description |
|---|---|---|
| `lobby_live_<random>` | API token | Server-side lookup key |
| `client_id` | UUID v4 | Unique client identifier |
| `from_address` | Ethereum address | Bound address for this key |

---

### 4.2 Authentication Flow

**Request:**
```http
POST /v1/transactions
Authorization: Bearer lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92
Content-Type: application/json
```

**Server-Side Validation:**
1. Parse `Authorization` header → extract API token (`lobby_live_abc123xyz`)
2. Lookup token in `ApiRegistry` → retrieve `ClientConfig { client_id, from_address }`
3. Attach `ClientConfig` to request as `AuthenticatedClient` extension
4. Downstream handlers validate `from` field matches `from_address`

**Error Responses:**

| HTTP Status | Condition |
|---|---|
| `401 Unauthorized` | Missing `Authorization` header |
| `401 Unauthorized` | Invalid API key format (not `Bearer <key>`) |
| `401 Unauthorized` | API token not found in registry |
| `403 Forbidden` | Transaction `from` address doesn't match API key's bound address |

---

### 4.3 Address Binding Security

Each API key is **permanently bound** to a single `from_address`. This ensures:

- **No cross-account signing** — Key for address A cannot sign transactions from address B
- **Audit trail** — Every transaction is traceable to a specific client account
- **Key rotation** — Compromised keys can be revoked without affecting other accounts

**Validation:**
```rust
// Server-side check in submit_transaction handler
if from_address != client_config.from_address {
    return Err(HandlerError::FromAddressMismatch {
        expected: client_config.from_address,
        actual: from_address,
    });
}
```

---

### 4.4 Obtaining API Keys

API keys are provisioned by the Lobby operator. For local development:

1. Add an entry to your `.env` file:
   ```bash
   LOBBY_API_KEY_1=lobby_live_mytoken:550e8400-e29b-41d4-a716-446655440000:0xYOUR_ADDRESS
   ```

2. Ensure the `from_address` matches an entry in `test_keys.json`:
   ```json
   {
     "account1": {
       "pvt_key": "0x...",
       "address": "0xYOUR_ADDRESS"
     }
   }
   ```

For production deployments, contact your Lobby operator for secure key provisioning.

---

## 5. API Endpoints Reference

### 5.1 Transaction Submission

**Endpoint:** `POST /v1/transactions`

**Headers:**
```http
Authorization: Bearer <api_key>
Content-Type: application/json
```

**Request Body:**
```json
{
  "jsonrpc": "2.0",
  "method": "eth_sendTransaction",
  "params": [{
    "from": "0xYOUR_ADDRESS",
    "to": "0xRECIPIENT_ADDRESS",
    "value": "0x2386f26fc10000",
    "data": "0x",
    "chainId": "0x1",
    "gas": "0x5208",
    "maxFeePerGas": "0xba43b7400",
    "maxPriorityFeePerGas": "0x77359400"
  }],
  "id": 1
}
```

**Field Specifications:**

| Field | Required | Type | Description |
|---|---|---|---|
| `jsonrpc` | ✅ Yes | string | Must be `"2.0"` |
| `method` | ✅ Yes | string | Must be `"eth_sendTransaction"` |
| `params` | ✅ Yes | array | Single transaction object |
| `from` | ✅ Yes | address | Must match API key's bound address |
| `to` | No | address \| null | Recipient (omit for contract creation) |
| `value` | No | hex uint256 | Wei amount (default: `"0x0"`) |
| `data` | No | hex bytes | Calldata (default: `"0x"`) |
| `chainId` | ✅ Yes | hex uint64 | Chain ID (see [Supported Chains](#102-supported-chains)) |
| `gas` | ✅ Yes | hex uint256 | Gas limit |
| `maxFeePerGas` | ✅ Yes | hex uint256 | Max fee per gas (EIP-1559) |
| `maxPriorityFeePerGas` | ✅ Yes | hex uint256 | Miner tip per gas (EIP-1559) |
| `id` | ✅ Yes | number \| string | Request ID (echoed in response) |

**Validation Rules:**

Lobby performs the following checks before accepting a transaction:

| Rule | Error Message |
|---|---|
| `gas` > 0 | `"gas limit cannot be zero"` |
| `gas` ≤ 30,000,000 | `"fee exceeds max gas limit: 30000000"` |
| `maxFeePerGas` > 0 | `"max_fee_per_gas cannot be zero"` |
| `maxPriorityFeePerGas` < `maxFeePerGas` | `"max_priority_fee_per_gas cannot exceed max_fee_per_gas"` |
| `chainId` in supported chains | `"unsupported chain_id"` |

**Success Response (202 Accepted):**
```json
{
  "jsonrpc": "2.0",
  "result": {
    "execution_id": "550e8400-e29b-41d4-a716-446655440000",
    "status": "accepted"
  },
  "id": 1
}
```

**Response Fields:**
| Field | Description |
|---|---|
| `execution_id` | UUID for status tracking — **save this immediately** |
| `status` | Always `"accepted"` on success |

**What Happens Next:**

1. Transaction is persisted by RelayHost actor
2. Pipeline semaphore permit acquired (backpressure control)
3. Background task spawned for Nonce → Sign → Broadcast → Validator pipeline
4. HTTP response returned immediately (pipeline runs asynchronously)

---

### 5.2 Status Polling

**Endpoint:** `GET /status/{execution_id}`

**Headers:**
```http
Authorization: Bearer <api_key>
```

**Path Parameter:**
| Name | Type | Description |
|---|---|---|
| `execution_id` | UUID v4 | Execution ID from submission response |

**Response Format:**

The response structure varies based on the current pipeline stage:

**In-Progress (permit_acquired, accepted, nonce_reserved, signed):**
```json
{
  "execution_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "nonce_reserved"
}
```

**Broadcasted:**
```json
{
  "execution_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "broadcasted",
  "tx_hash": "0xabc123...",
  "message": "Waiting for validation on-chain"
}
```

**Confirmed (Terminal State):**
```json
{
  "execution_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "confirmed_on_chain",
  "tx_hash": "0xabc123..."
}
```

**Failed (Terminal State):**
```json
{
  "execution_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "failed",
  "stage": "broadcast",
  "reason": "Provider error: insufficient funds"
}
```

**Nonce Mismatch Detected (Transient State):**
```json
{
  "execution_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "nonce_mismatch_detected",
  "nonce_on_chain": "42",
  "attempted_nonce": "41"
}
```
*This state indicates Lobby is syncing with the blockchain. The next poll will show either `signed` (after re-signing) or `failed`.*

**Validator Timed Out (Terminal State):**
```json
{
  "execution_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "validator_timed_out",
  "message": "validator timed out, wait for confirmation"
}
```
*Scanner Bot will continue polling RPC. Check status again in a few minutes.*

**Error Response (404 Not Found):**
```json
{
  "jsonrpc": "2.0",
  "error": {
    "code": -32001,
    "message": "no pipeline record found for the given execution id: 550e8400-..."
  },
  "id": null
}
```

---

## 6. Transaction Lifecycle & Status Tracking

### 6.1 Status State Machine

```
                    ┌──────────────────┐
                    │  permit_acquired │
                    └────────┬─────────┘
                             │
                             ▼
                    ┌──────────────────┐
                    │     accepted     │
                    └────────┬─────────┘
                             │
                             ▼
                    ┌──────────────────┐
                    │  nonce_reserved  │
                    └────────┬─────────┘
                             │
                             ▼
                    ┌──────────────────┐
                    │      signed      │
                    └────────┬─────────┘
                             │
                             ▼
              ┌──────────────┴──────────────┐
              │                             │
              ▼                             ▼
  ┌──────────────────────┐      ┌──────────────────────┐
  │ nonce_mismatch_      │      │     broadcasted      │
  │ detected             │      │                      │
  └──────────┬───────────┘      └──────────┬───────────┘
             │                             │
             │ (auto-recovery)             │
             │                             ▼
             └────────►          ┌──────────────────────┐
                                 │  confirmed_on_chain  │ ✓
                                 └──────────────────────┘
                                           │
                         ┌─────────────────┼─────────────────┐
                         │                 │                 │
                         ▼                 ▼                 ▼
               ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
               │    failed    │  │validator_    │  │   failed     │
               │              │  │timed_out     │  │              │
               └──────────────┘  └──────────────┘  └──────────────┘
                      ✗                 ⏳                 ✗
```

### 6.2 Status Descriptions

| Status | Description | Terminal? | Typical Duration |
|---|---|---|---|
| `permit_acquired` | Pipeline concurrency permit acquired | No | < 1ms |
| `accepted` | RelayHost validated and persisted transaction | No | 10-50ms |
| `nonce_reserved` | Nonce assigned and locked | No | 50-200ms |
| `signed` | Transaction signed with private key | No | 10-50ms |
| `broadcasted` | Submitted to blockchain RPC node | No | 30-300s |
| `nonce_mismatch_detected` | On-chain nonce differs from database (auto-recovering) | No | 1-3s |
| `confirmed_on_chain` | ≥3 block confirmations received | ✅ Yes | 30-300s |
| `failed` | Pipeline failed at specific stage | ✅ Yes | Variable |
| `validator_timed_out` | Validator gave up after 5 minutes (Scanner Bot continues) | ✅ Yes | 300s |

### 6.3 Polling Strategy

**Recommended Approach:**

```python
def poll_transaction(execution_id: str) -> dict:
    """
    Poll transaction status until terminal state reached.
    
    Returns final status dict on success/failure.
    Raises TimeoutError if no terminal state after 5 minutes.
    """
    start = time.time()
    
    while time.time() - start < 300:  # 5 minute timeout
        status = get_status(execution_id)
        
        # Terminal states
        if status["status"] in ["confirmed_on_chain", "failed"]:
            return status
        
        # Validator timeout (terminal, but Scanner Bot may recover later)
        if status["status"] == "validator_timed_out":
            return status
        
        # Still processing
        time.sleep(3)  # Poll every 3 seconds
    
    raise TimeoutError("Transaction did not reach terminal state")
```

**Polling Frequency:**
- **Interval:** 3-5 seconds
- **Timeout:** 5 minutes (matches Lobby's validator timeout)
- **Backoff:** Not necessary (constant interval is fine)

**State Retention:**

> ⚠️ **Prototype Behavior:** Status entries currently persist in-memory indefinitely (until Lobby restarts). Production versions will use Redis persistence with configurable TTL.

---

### 6.4 Terminal State Handling

**Confirmed Transaction:**
```json
{
  "status": "confirmed_on_chain",
  "tx_hash": "0xabc123..."
}
```

**Action:** Transaction succeeded. You can:
- View on block explorer (Etherscan, Polygonscan, etc.)
- Stop polling
- Store `tx_hash` for record-keeping

**Failed Transaction:**
```json
{
  "status": "failed",
  "stage": "broadcast",
  "reason": "Provider error: insufficient funds"
}
```

**Action:** Transaction failed. Common failure reasons:

| Stage | Reason | Fix |
|---|---|---|
| `relay_host` | Validation failed | Check transaction parameters (gas, fees, chain ID) |
| `nonce_reserve` | Database error | Retry submission (transient DB issue) |
| `sign` | Signing failed | Contact Lobby operator (key custody issue) |
| `broadcast` | Insufficient funds | Add funds to `from_address` |
| `broadcast` | MissingProvider | Chain not configured (contact operator) |
| `validator` | Transaction reverted | Check contract logic / calldata |

**Validator Timeout:**
```json
{
  "status": "validator_timed_out",
  "message": "validator timed out, wait for confirmation"
}
```

**Action:** Transaction was broadcast but validator gave up waiting. This can happen if:
- High network congestion (transaction stuck in mempool)
- Nonce gap created by external wallet (transaction blocked behind missing nonce)
- RPC node temporarily unavailable

**What to do:**
- Wait 5-10 minutes and poll status again (Scanner Bot may find the transaction)
- Check block explorer manually for `tx_hash` (if available in earlier `broadcasted` status)
- If transaction is truly lost, you can safely resubmit (nonce has been marked `consumed`)

---

## 7. Error Handling

### 7.1 JSON-RPC Error Format

All errors follow the JSON-RPC 2.0 specification:

```json
{
  "jsonrpc": "2.0",
  "error": {
    "code": -32602,
    "message": "Human-readable error description",
    "data": {
      "field": "optional extra context"
    }
  },
  "id": 1
}
```

### 7.2 Error Code Reference

| HTTP Status | JSON-RPC Code | Error Type | Action |
|---|---|---|---|
| 400 | `-32600` | Invalid JSON-RPC version | Set `jsonrpc` to `"2.0"` |
| 400 | `-32601` | Unsupported method | Use `"eth_sendTransaction"` |
| 400 | `-32602` | Invalid/missing params | Check required fields and types |
| 401 | (custom) | Missing auth header | Add `Authorization: Bearer <key>` |
| 401 | (custom) | Invalid API key format | Check key format: `lobby_live_<token>:<client_id>:<address>` |
| 401 | (custom) | Invalid API key | Verify API key with Lobby operator |
| 403 | `-32000` | From address mismatch | Use address bound to your API key |
| 404 | `-32001` | Execution ID not found | Verify `execution_id` is correct |
| 500 | `-32603` | Internal server error | Retry with exponential backoff |

### 7.3 Common Error Scenarios

**Invalid Transaction Parameters:**
```json
{
  "error": {
    "code": -32602,
    "message": "Invalid params: lint error: max_priority_fee_per_gas cannot exceed max_fee_per_gas"
  }
}
```
**Fix:** Ensure `maxPriorityFeePerGas` < `maxFeePerGas`

**Address Mismatch:**
```json
{
  "error": {
    "code": -32000,
    "message": "From address does not match authenticated account",
    "data": {
      "expected": "0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92",
      "actual": "0x5b35c4571b935076c853e9c3aab59b60d7b32daf"
    }
  }
}
```
**Fix:** Use the `from_address` bound to your API key

**Unsupported Chain:**
```json
{
  "error": {
    "code": -32602,
    "message": "Invalid params: lint error: unsupported chain_id"
  }
}
```
**Fix:** See [Supported Chains](#102-supported-chains) for valid chain IDs

**Missing RPC Provider:**
```json
{
  "error": {
    "code": -32603,
    "message": "Internal pipeline error"
  }
}
```
**Server logs will show:** `Provider error: ChainId(56)` (BSC not configured)

**Fix:** Contact Lobby operator to add RPC endpoint for desired chain

---

### 7.4 Client-Side Retry Logic

**Retryable Errors (HTTP 500, 503):**
```python
import time

def submit_with_retry(transaction, max_retries=3):
    for attempt in range(max_retries):
        try:
            response = submit_transaction(transaction)
            return response
        except ServerError as e:
            if e.status_code in [500, 503] and attempt < max_retries - 1:
                delay = min(2 ** attempt, 10)  # Exponential backoff (cap at 10s)
                time.sleep(delay)
                continue
            raise
```

**Non-Retryable Errors (4xx):**

Do **not** retry client errors (400, 401, 403, 404). These indicate:
- Invalid request format → Fix your code
- Authentication failure → Fix your API key
- Validation failure → Fix transaction parameters

**Idempotency Considerations:**

Lobby generates a unique `execution_id` for every submission. If you retry a failed submission, you will create a **new transaction** with a **new execution ID**.

To avoid duplicate submissions:
1. Store the `execution_id` immediately after receiving `202 Accepted`
2. On retry, poll the original `execution_id` first to check if it's still processing
3. Only resubmit if the original transaction reached a terminal `failed` state

---

## 8. Client Implementation Examples

### 8.1 Python Implementation

**Full-featured client with transaction submission and status polling:**

```python
import requests
import time
from typing import Dict, Any, Optional
from dataclasses import dataclass

@dataclass
class LobbyConfig:
    base_url: str
    api_key: str
    from_address: str

class LobbyClient:
    """
    High-level client for Lobby transaction service.
    
    Example usage:
        client = LobbyClient(LobbyConfig(
            base_url="http://localhost:3000",
            api_key="lobby_live_...",
            from_address="0xaf9ce11..."
        ))
        
        # Submit transaction
        exec_id = client.submit_transaction(
            to="0x5b35c4...",
            value_wei=100_000_000_000_000_000,  # 0.1 ETH
            chain_id=1
        )
        
        # Wait for confirmation
        result = client.wait_for_confirmation(exec_id, timeout=300)
        print(f"Transaction confirmed: {result['tx_hash']}")
    """
    
    def __init__(self, config: LobbyConfig):
        self.config = config
        self.session = requests.Session()
        self.session.headers.update({
            "Authorization": f"Bearer {config.api_key}",
            "Content-Type": "application/json"
        })
    
    def submit_transaction(
        self,
        to: str,
        value_wei: int = 0,
        data: str = "0x",
        chain_id: int = 1,
        gas: int = 21000,
        max_fee_per_gas: int = 50_000_000_000,
        max_priority_fee_per_gas: int = 2_000_000_000
    ) -> str:
        """
        Submit a transaction to Lobby.
        
        Returns:
            execution_id (str): UUID for status tracking
            
        Raises:
            requests.HTTPError: On 4xx/5xx response
            ValueError: On JSON-RPC error response
        """
        response = self.session.post(
            f"{self.config.base_url}/v1/transactions",
            json={
                "jsonrpc": "2.0",
                "method": "eth_sendTransaction",
                "params": [{
                    "from": self.config.from_address,
                    "to": to,
                    "value": hex(value_wei),
                    "data": data,
                    "chainId": hex(chain_id),
                    "gas": hex(gas),
                    "maxFeePerGas": hex(max_fee_per_gas),
                    "maxPriorityFeePerGas": hex(max_priority_fee_per_gas)
                }],
                "id": 1
            }
        )
        
        response.raise_for_status()
        body = response.json()
        
        if "error" in body:
            raise ValueError(
                f"Lobby error {body['error']['code']}: {body['error']['message']}"
            )
        
        execution_id = body["result"]["execution_id"]
        print(f"✓ Transaction accepted: {execution_id}")
        return execution_id
    
    def get_status(self, execution_id: str) -> Dict[str, Any]:
        """
        Query transaction status.
        
        Returns:
            Status dict with keys: execution_id, status, [tx_hash], [stage], [reason]
        """
        response = self.session.get(
            f"{self.config.base_url}/status/{execution_id}"
        )
        
        if response.status_code == 404:
            raise ValueError(f"Execution ID not found: {execution_id}")
        
        response.raise_for_status()
        return response.json()
    
    def wait_for_confirmation(
        self,
        execution_id: str,
        poll_interval: float = 3.0,
        timeout: float = 300.0,
        verbose: bool = True
    ) -> Dict[str, Any]:
        """
        Poll status until transaction reaches terminal state.
        
        Args:
            execution_id: UUID from submit_transaction()
            poll_interval: Seconds between status checks (default: 3s)
            timeout: Max wait time in seconds (default: 300s)
            verbose: Print status updates (default: True)
        
        Returns:
            Final status dict
            
        Raises:
            TimeoutError: If timeout exceeded before terminal state
        """
        start_time = time.time()
        terminal_states = {"confirmed_on_chain", "failed", "validator_timed_out"}
        
        while True:
            elapsed = time.time() - start_time
            
            if elapsed > timeout:
                raise TimeoutError(
                    f"Transaction {execution_id} did not complete within {timeout}s"
                )
            
            status = self.get_status(execution_id)
            state = status["status"]
            
            if verbose:
                log = f"[{elapsed:.1f}s] {state}"
                if "tx_hash" in status:
                    log += f" | {status['tx_hash']}"
                print(log)
            
            # Check terminal states
            if state in terminal_states:
                if state == "confirmed_on_chain":
                    print(f"✓ Confirmed: {status['tx_hash']}")
                elif state == "failed":
                    print(f"✗ Failed at {status['stage']}: {status['reason']}")
                else:  # validator_timed_out
                    print(f"⏳ Validator timeout: {status['message']}")
                
                return status
            
            time.sleep(poll_interval)


# ============================================================
# Example Usage
# ============================================================

if __name__ == "__main__":
    client = LobbyClient(LobbyConfig(
        base_url="http://localhost:3000",
        api_key="lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92",
        from_address="0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92"
    ))
    
    # Submit ETH transfer
    exec_id = client.submit_transaction(
        to="0x5b35c4571b935076c853e9c3aab59b60d7b32daf",
        value_wei=100_000_000_000_000_000,  # 0.1 ETH
        chain_id=1,  # Ethereum mainnet
        gas=21000,
        max_fee_per_gas=50_000_000_000,       # 50 gwei
        max_priority_fee_per_gas=2_000_000_000  # 2 gwei
    )
    
    # Wait for confirmation
    try:
        result = client.wait_for_confirmation(exec_id, timeout=300)
        
        if result["status"] == "confirmed_on_chain":
            print(f"\nSuccess! Etherscan: https://etherscan.io/tx/{result['tx_hash']}")
        else:
            print(f"\nTransaction did not confirm. Final status: {result['status']}")
    
    except TimeoutError as e:
        print(f"\nTimeout: {e}")
        print("Check status later or view on block explorer")
```

**Output Example:**
```
✓ Transaction accepted: 9d3f7b2a-4c8e-4a1b-9f6d-8e5c3b2a1d0f
[0.1s] accepted
[3.2s] nonce_reserved
[6.3s] signed
[9.5s] broadcasted | 0xabc123def456...
[12.6s] broadcasted | 0xabc123def456...
[18.9s] confirmed_on_chain | 0xabc123def456...
✓ Confirmed: 0xabc123def456...

Success! Etherscan: https://etherscan.io/tx/0xabc123def456...
```

---

### 8.2 TypeScript Implementation

**Full-featured client with TypeScript types and error handling:**

```typescript
// ============================================================
// Types
// ============================================================

interface LobbyConfig {
  baseUrl: string;
  apiKey: string;
  fromAddress: string;
}

interface TransactionParams {
  to: string;
  value?: bigint;
  data?: string;
  chainId: number;
  gas: bigint;
  maxFeePerGas: bigint;
  maxPriorityFeePerGas: bigint;
}

interface StatusResponse {
  execution_id: string;
  status: string;
  tx_hash?: string;
  stage?: string;
  reason?: string;
  message?: string;
  nonce_on_chain?: string;
  attempted_nonce?: string;
}

interface JsonRpcError {
  code: number;
  message: string;
  data?: any;
}

// ============================================================
// Client Implementation
// ============================================================

class LobbyClient {
  private config: LobbyConfig;
  private headers: Record<string, string>;

  constructor(config: LobbyConfig) {
    this.config = config;
    this.headers = {
      "Authorization": `Bearer ${config.apiKey}`,
      "Content-Type": "application/json"
    };
  }

  /**
   * Submit a transaction to Lobby.
   * 
   * @returns Execution ID for status tracking
   * @throws Error on HTTP failure or JSON-RPC error
   */
  async submitTransaction(params: TransactionParams): Promise<string> {
    const response = await fetch(`${this.config.baseUrl}/v1/transactions`, {
      method: "POST",
      headers: this.headers,
      body: JSON.stringify({
        jsonrpc: "2.0",
        method: "eth_sendTransaction",
        params: [{
          from: this.config.fromAddress,
          to: params.to,
          value: `0x${(params.value ?? 0n).toString(16)}`,
          data: params.data ?? "0x",
          chainId: `0x${params.chainId.toString(16)}`,
          gas: `0x${params.gas.toString(16)}`,
          maxFeePerGas: `0x${params.maxFeePerGas.toString(16)}`,
          maxPriorityFeePerGas: `0x${params.maxPriorityFeePerGas.toString(16)}`
        }],
        id: 1
      })
    });

    if (!response.ok) {
      throw new Error(`HTTP ${response.status}: ${response.statusText}`);
    }

    const body = await response.json();

    if (body.error) {
      const err = body.error as JsonRpcError;
      throw new Error(`Lobby error ${err.code}: ${err.message}`);
    }

    const executionId = body.result.execution_id;
    console.log(`✓ Transaction accepted: ${executionId}`);
    return executionId;
  }

  /**
   * Query transaction status.
   * 
   * @throws Error on HTTP failure
   */
  async getStatus(executionId: string): Promise<StatusResponse> {
    const response = await fetch(
      `${this.config.baseUrl}/status/${executionId}`,
      { headers: this.headers }
    );

    if (response.status === 404) {
      throw new Error(`Execution ID not found: ${executionId}`);
    }

    if (!response.ok) {
      throw new Error(`HTTP ${response.status}: ${response.statusText}`);
    }

    return response.json();
  }

  /**
   * Poll status until terminal state reached.
   * 
   * @param executionId UUID from submitTransaction()
   * @param pollInterval Milliseconds between checks (default: 3000)
   * @param timeout Max wait time in milliseconds (default: 300000)
   * @param verbose Print status updates (default: true)
   * 
   * @returns Final status object
   * @throws Error on timeout or HTTP failure
   */
  async waitForConfirmation(
    executionId: string,
    pollInterval: number = 3000,
    timeout: number = 300000,
    verbose: boolean = true
  ): Promise<StatusResponse> {
    const startTime = Date.now();
    const terminalStates = new Set([
      "confirmed_on_chain",
      "failed",
      "validator_timed_out"
    ]);

    while (true) {
      const elapsed = Date.now() - startTime;

      if (elapsed > timeout) {
        throw new Error(
          `Transaction ${executionId} did not complete within ${timeout / 1000}s`
        );
      }

      const status = await this.getStatus(executionId);

      if (verbose) {
        let log = `[${(elapsed / 1000).toFixed(1)}s] ${status.status}`;
        if (status.tx_hash) {
          log += ` | ${status.tx_hash}`;
        }
        console.log(log);
      }

      // Check terminal states
      if (terminalStates.has(status.status)) {
        if (status.status === "confirmed_on_chain") {
          console.log(`✓ Confirmed: ${status.tx_hash}`);
        } else if (status.status === "failed") {
          console.log(`✗ Failed at ${status.stage}: ${status.reason}`);
        } else {
          console.log(`⏳ Validator timeout: ${status.message}`);
        }
        return status;
      }

      await new Promise(resolve => setTimeout(resolve, pollInterval));
    }
  }
}

// ============================================================
// Example Usage
// ============================================================

const client = new LobbyClient({
  baseUrl: "http://localhost:3000",
  apiKey: "lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92",
  fromAddress: "0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92"
});

// Submit ETH transfer
const executionId = await client.submitTransaction({
  to: "0x5b35c4571b935076c853e9c3aab59b60d7b32daf",
  value: 100_000_000_000_000_000n, // 0.1 ETH
  chainId: 1, // Ethereum mainnet
  gas: 21000n,
  maxFeePerGas: 50_000_000_000n,       // 50 gwei
  maxPriorityFeePerGas: 2_000_000_000n // 2 gwei
});

// Wait for confirmation
try {
  const result = await client.waitForConfirmation(executionId, 3000, 300000);

  if (result.status === "confirmed_on_chain") {
    console.log(`\nSuccess! Etherscan: https://etherscan.io/tx/${result.tx_hash}`);
  } else {
    console.log(`\nTransaction did not confirm. Final status: ${result.status}`);
  }
} catch (error) {
  console.error(`\nError: ${error.message}`);
  console.log("Check status later or view on block explorer");
}
```

---

### 8.3 Rust Implementation

**Full-featured async client with strong typing and error handling:**

```rust
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::time::sleep;

// ============================================================
// Configuration
// ============================================================

#[derive(Clone)]
pub struct LobbyConfig {
    pub base_url: String,
    pub api_key: String,
    pub from_address: String,
}

// ============================================================
// Request/Response Types
// ============================================================

#[derive(Serialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    method: String,
    params: Vec<TransactionParams>,
    id: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TransactionParams {
    from: String,
    to: String,
    value: String,
    data: String,
    chain_id: String,
    gas: String,
    max_fee_per_gas: String,
    max_priority_fee_per_gas: String,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    result: Option<SubmitResult>,
    error: Option<JsonRpcError>,
}

#[derive(Deserialize)]
struct SubmitResult {
    execution_id: String,
}

#[derive(Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct StatusResponse {
    pub execution_id: String,
    pub status: String,
    pub tx_hash: Option<String>,
    pub stage: Option<String>,
    pub reason: Option<String>,
    pub message: Option<String>,
}

// ============================================================
// Client Implementation
// ============================================================

pub struct LobbyClient {
    config: LobbyConfig,
    client: Client,
}

impl LobbyClient {
    pub fn new(config: LobbyConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }

    /// Submit a transaction to Lobby.
    ///
    /// # Returns
    /// Execution ID (UUID) for status tracking
    ///
    /// # Errors
    /// Returns error on HTTP failure or JSON-RPC error response
    pub async fn submit_transaction(
        &self,
        to: &str,
        value_wei: u128,
        chain_id: u64,
        gas: u64,
        max_fee_per_gas: u128,
        max_priority_fee_per_gas: u128,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "eth_sendTransaction".to_string(),
            params: vec![TransactionParams {
                from: self.config.from_address.clone(),
                to: to.to_string(),
                value: format!("0x{:x}", value_wei),
                data: "0x".to_string(),
                chain_id: format!("0x{:x}", chain_id),
                gas: format!("0x{:x}", gas),
                max_fee_per_gas: format!("0x{:x}", max_fee_per_gas),
                max_priority_fee_per_gas: format!("0x{:x}", max_priority_fee_per_gas),
            }],
            id: 1,
        };

        let response: JsonRpcResponse = self
            .client
            .post(format!("{}/v1/transactions", self.config.base_url))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&request)
            .send()
            .await?
            .json()
            .await?;

        if let Some(err) = response.error {
            return Err(format!("Lobby error {}: {}", err.code, err.message).into());
        }

        let execution_id = response
            .result
            .ok_or("Missing result in response")?
            .execution_id;

        println!("✓ Transaction accepted: {}", execution_id);
        Ok(execution_id)
    }

    /// Query transaction status.
    pub async fn get_status(
        &self,
        execution_id: &str,
    ) -> Result<StatusResponse, Box<dyn std::error::Error>> {
        let response = self
            .client
            .get(format!("{}/status/{}", self.config.base_url, execution_id))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .send()
            .await?;

        if response.status() == 404 {
            return Err(format!("Execution ID not found: {}", execution_id).into());
        }

        Ok(response.json().await?)
    }

    /// Poll status until terminal state reached.
    ///
    /// # Arguments
    /// * `execution_id` - UUID from submit_transaction()
    /// * `poll_interval` - Duration between status checks
    /// * `timeout` - Maximum wait duration
    /// * `verbose` - Print status updates
    ///
    /// # Returns
    /// Final status response
    ///
    /// # Errors
    /// Returns error on timeout or HTTP failure
    pub async fn wait_for_confirmation(
        &self,
        execution_id: &str,
        poll_interval: Duration,
        timeout: Duration,
        verbose: bool,
    ) -> Result<StatusResponse, Box<dyn std::error::Error>> {
        let start_time = Instant::now();
        let terminal_states = ["confirmed_on_chain", "failed", "validator_timed_out"];

        loop {
            let elapsed = start_time.elapsed();

            if elapsed > timeout {
                return Err(format!(
                    "Transaction {} did not complete within {:?}",
                    execution_id, timeout
                )
                .into());
            }

            let status = self.get_status(execution_id).await?;

            if verbose {
                let mut log = format!("[{:.1}s] {}", elapsed.as_secs_f64(), status.status);
                if let Some(ref tx_hash) = status.tx_hash {
                    log.push_str(&format!(" | {}", tx_hash));
                }
                println!("{}", log);
            }

            // Check terminal states
            if terminal_states.contains(&status.status.as_str()) {
                match status.status.as_str() {
                    "confirmed_on_chain" => {
                        println!(
                            "✓ Confirmed: {}",
                            status.tx_hash.as_ref().unwrap_or(&"N/A".to_string())
                        );
                    }
                    "failed" => {
                        println!(
                            "✗ Failed at {}: {}",
                            status.stage.as_ref().unwrap_or(&"unknown".to_string()),
                            status.reason.as_ref().unwrap_or(&"unknown".to_string())
                        );
                    }
                    "validator_timed_out" => {
                        println!(
                            "⏳ Validator timeout: {}",
                            status.message.as_ref().unwrap_or(&"N/A".to_string())
                        );
                    }
                    _ => {}
                }
                return Ok(status);
            }

            sleep(poll_interval).await;
        }
    }
}

// ============================================================
// Example Usage
// ============================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = LobbyClient::new(LobbyConfig {
        base_url: "http://localhost:3000".to_string(),
        api_key: "lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92".to_string(),
        from_address: "0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92".to_string(),
    });

    // Submit ETH transfer
    let execution_id = client
        .submit_transaction(
            "0x5b35c4571b935076c853e9c3aab59b60d7b32daf", // to
            100_000_000_000_000_000,                        // 0.1 ETH in wei
            1,                                              // Ethereum mainnet
            21000,                                          // gas
            50_000_000_000,                                 // 50 gwei max fee
            2_000_000_000,                                  // 2 gwei priority fee
        )
        .await?;

    // Wait for confirmation
    match client
        .wait_for_confirmation(
            &execution_id,
            Duration::from_secs(3),
            Duration::from_secs(300),
            true,
        )
        .await
    {
        Ok(result) => {
            if result.status == "confirmed_on_chain" {
                println!(
                    "\nSuccess! Etherscan: https://etherscan.io/tx/{}",
                    result.tx_hash.unwrap()
                );
            } else {
                println!("\nTransaction did not confirm. Final status: {}", result.status);
            }
        }
        Err(e) => {
            eprintln!("\nError: {}", e);
            println!("Check status later or view on block explorer");
        }
    }

    Ok(())
}
```

---

## 9. Production Considerations

### 9.1 Security Best Practices

**API Key Management:**
- **Never commit API keys to version control** — Use environment variables or secret management services (AWS Secrets Manager, HashiCorp Vault, etc.)
- **Rotate keys periodically** — Lobby operators should provide key rotation mechanisms
- **Use separate keys per environment** — Different keys for dev, staging, and production
- **Monitor key usage** — Track which keys are making requests (future Lobby feature)

**Private Key Custody:**

> ⚠️ **Critical:** The current `JsonPolicyEngine` stores private keys in plaintext JSON files. This is **acceptable only for local development**.

Production deployments **must** use:
- **Hardware Security Modules (HSM)** — Dedicated cryptographic hardware
- **Cloud KMS** — AWS KMS, Google Cloud KMS, Azure Key Vault
- **Secure enclaves** — Intel SGX, ARM TrustZone

Future Lobby versions will support pluggable key custody backends.

---

### 9.2 Rate Limiting & Backpressure

**Pipeline Semaphore:**

Lobby enforces concurrency limits via a semaphore (default: 17 concurrent pipelines). If this limit is exceeded:

```json
{
  "error": {
    "code": -32603,
    "message": "pipeline semaphore timed out after 5000ms — server is overloaded"
  }
}
```

**Client-side response:**
- Return `HTTP 429 Too Many Requests` to your end users
- Implement exponential backoff before retrying
- Consider implementing client-side request queuing

**Production tuning:**

Adjust semaphore size via environment variable:
```bash
PIPELINE_CONCURRENCY=50  # Allow 50 concurrent transactions
```

**Capacity planning:**

Lobby's throughput depends on:
- **RPC provider limits** — Alchemy/Infura typically cap at 10-50 req/s per tier
- **Database connections** — PostgreSQL connection pool (default: 17)
- **Actor shard count** — More shards = higher parallelism (diminishing returns after ~32)

---

### 9.3 Monitoring & Observability

**Structured Logging:**

Lobby emits structured logs (via `tracing` crate) for every pipeline stage:

```
2026-03-21T12:34:56.789Z INFO  pipeline{execution_id=550e8400... chain_id=1 from_address=0xaf9ce1...}:
  elapsed_ms=123 nonce=42 "nonce reserved"
```

**Key metrics to track:**

| Metric | Description | Alert Threshold |
|---|---|---|
| Pipeline latency | Time from submission to confirmation | > 60s (P95) |
| Nonce reservation failures | Database or sync errors | > 1% error rate |
| Broadcast failures | RPC rejections or timeouts | > 5% error rate |
| Validator timeouts | Transactions not confirmed after 5 min | > 10% timeout rate |
| Stale nonce count | Reserved nonces > 2 minutes old | > 10 active |

**Recommended tools:**
- **Prometheus** — Metrics collection (future Lobby integration)
- **Grafana** — Dashboards and alerting
- **Datadog / New Relic** — APM and distributed tracing
- **Sentry** — Error tracking and aggregation

---

### 9.4 Database Maintenance

**PostgreSQL Tuning:**

Lobby's database schema uses:
- **Indexed queries** on `(chain_id, from_address, execution_id)`
- **Frequent writes** to `nonce_assignments` and `sign_requests` tables
- **Periodic scans** by Sweeper and Scanner bots

**Recommended settings:**
```sql
-- Connection pooling
max_connections = 100
shared_buffers = 256MB

-- Write optimization
synchronous_commit = off  -- Accept risk of 1-2 second data loss on crash
wal_buffers = 16MB

-- Index efficiency
random_page_cost = 1.1  -- SSD storage assumed
```

**Backup strategy:**
- **Continuous archiving** — PostgreSQL WAL archiving to S3/GCS
- **Point-in-time recovery** — Daily full backups + WAL replay
- **Retention policy** — 7 days for transaction records, 30 days for audit logs

---

### 9.5 High Availability

**Redundancy:**

Lobby currently runs as a single-instance service. For production HA:

1. **Database replication** — PostgreSQL streaming replication (primary + standby)
2. **Load balancer** — Distribute traffic across multiple Lobby instances
3. **Shared state** — Redis for StatusRegistry (already implemented)
4. **Stateless actors** — Nonce/Sign/Broadcast actors can run on any instance

**Deployment architecture (future):**

```
                    ┌──────────────┐
                    │ Load Balancer│
                    └───────┬──────┘
                            │
        ┌───────────────────┼───────────────────┐
        ▼                   ▼                   ▼
    ┌────────┐          ┌────────┐          ┌────────┐
    │ Lobby  │          │ Lobby  │          │ Lobby  │
    │Instance│          │Instance│          │Instance│
    │   1    │          │   2    │          │   3    │
    └────┬───┘          └────┬───┘          └────┬───┘
         │                   │                   │
         └───────────────────┼───────────────────┘
                             ▼
                    ┌─────────────────┐
                    │  PostgreSQL HA  │
                    │ (Primary+Standby)│
                    └─────────────────┘
                             ▼
                    ┌─────────────────┐
                    │  Redis Cluster  │
                    │ (StatusRegistry)│
                    └─────────────────┘
```

---

## 10. Appendices

### 10.1 Glossary

| Term | Definition |
|---|---|
| **Execution ID** | UUID v4 identifier assigned to every transaction submission |
| **Nonce** | Sequential transaction counter per Ethereum account |
| **EIP-1559** | Ethereum Improvement Proposal introducing base fee + priority fee gas model |
| **RLP** | Recursive Length Prefix — Ethereum's binary serialization format |
| **Actor** | Isolated async task with message-passing interface (Tokio async/await) |
| **Shard** | Partition of key-space routed to specific actor instance |
| **Semaphore** | Concurrency control primitive (limits max parallel pipelines) |
| **RelayHost** | Pipeline stage that validates and persists transaction intents |
| **Cortex** | Central orchestrator that coordinates the five-stage pipeline |

---

### 10.2 Supported Chains

| Chain | Chain ID (dec) | Chain ID (hex) | Explorer |
|---|---|---|---|
| **Ethereum Mainnet** | 1 | `0x1` | [etherscan.io](https://etherscan.io) |
| **Hoodi Testnet** | 560048 | `0x88bb0` | [hoodi.blockscout.com](https://hoodi.blockscout.com) |
| **Polygon** | 137 | `0x89` | [polygonscan.com](https://polygonscan.com) |
| **Arbitrum One** | 42161 | `0xa4b1` | [arbiscan.io](https://arbiscan.io) |

> **Note:** Sepolia testnet (chain ID 11155111) was originally supported but will be deprecated by end of 2026. Hoodi testnet is the recommended replacement for development.

**Adding new chains:**

Contact your Lobby operator to configure RPC endpoints for additional chains. The operator must set:

```bash
RPC_ENDPOINT_<chain_id>=https://<rpc_url>
```

---

### 10.3 Gas Parameter Guidelines

Lobby **does not** currently support automatic gas estimation. You must provide:
- `gas` — Gas limit for transaction execution
- `maxFeePerGas` — Maximum total fee per gas unit (EIP-1559)
- `maxPriorityFeePerGas` — Miner tip per gas unit (EIP-1559)

**Recommended approach:**

1. **Use RPC `eth_estimateGas`** to determine appropriate `gas` limit
2. **Query `eth_feeHistory`** or gas tracker APIs (Etherscan, Blocknative) for current base fee
3. **Set fees:**
   - `maxPriorityFeePerGas`: 1-3 gwei (normal priority) or 5-10 gwei (high priority)
   - `maxFeePerGas`: `(current_base_fee × 2) + maxPriorityFeePerGas`

**Example calculation:**

```python
# Current network state
current_base_fee = 30_000_000_000  # 30 gwei

# Conservative settings (fast inclusion)
max_priority_fee = 2_000_000_000   # 2 gwei tip
max_fee = (current_base_fee * 2) + max_priority_fee
# Result: 62 gwei max fee

# Submit to Lobby
client.submit_transaction(
    to="0x...",
    gas=21000,
    max_fee_per_gas=max_fee,
    max_priority_fee_per_gas=max_priority_fee
)
```

**Common gas limits:**

| Operation | Typical Gas Limit |
|---|---|
| Simple ETH transfer | 21,000 |
| ERC-20 transfer | 65,000 |
| Uniswap swap | 150,000 - 300,000 |
| NFT mint | 100,000 - 500,000 |
| Complex DeFi interaction | 500,000 - 1,000,000 |

---

### 10.4 Running Lobby Locally

**Quick Start:**

```bash
# 1. Clone repository
git clone https://github.com/romanticNomad/Lobby.git
cd lobby

# 2. Start PostgreSQL (Docker)
docker run -d \
  --name lobby-postgres \
  -e POSTGRES_PASSWORD=password \
  -e POSTGRES_DB=lobby \
  -p 5432:5432 \
  postgres:15

# 3. Configure environment
cat > .env << EOF
DATABASE_URL=postgresql://postgres:password@localhost/lobby
SERVER_ADDR=0.0.0.0:3000
RUST_LOG=info

# API key (example)
LOBBY_API_KEY_1=lobby_live_dev123:550e8400-e29b-41d4-a716-446655440000:0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92

# RPC endpoints (get keys from Alchemy/Infura)
RPC_ENDPOINT_1=https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY
RPC_ENDPOINT_560048=https://eth-hoodi.g.alchemy.com/v2/YOUR_KEY
EOF

# 4. Create test keys
cat > test_keys.json << EOF
{
  "test_account": {
    "pvt_key": "0xe74176dc8bcf2e5e6500a8f117a665ed44bcf448206c0cd23cb1228e61a2729c",
    "pub_key": "0x045ae5fcf5e6e9fafbec1155a363467bdea7eeb453bf1e68944b873f81871344dc38209d99d4af74dcbcba7da410353cbd45241cc9e91b863c170a8b1bda7e78a3",
    "address": "0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92"
  }
}
EOF

# 5. Run migrations
source .env
DATABASE_URL=postgresql://postgres:password@localhost/lobby \
  cargo sqlx migrate run

# 6. Start Lobby
cargo run --release

# 7. Test submission
curl -X POST http://localhost:3000/v1/transactions \
  -H "Authorization: Bearer lobby_live_dev123:550e8400-e29b-41d4-a716-446655440000:0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_sendTransaction",
    "params": [{
      "from": "0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92",
      "to": "0x5b35c4571b935076c853e9c3aab59b60d7b32daf",
      "value": "0x2386f26fc10000",
      "chainId": "0x88bb0",
      "gas": "0x5208",
      "maxFeePerGas": "0xba43b7400",
      "maxPriorityFeePerGas": "0x77359400"
    }],
    "id": 1
  }'
```

**Expected output:**
```json
{
  "jsonrpc": "2.0",
  "result": {
    "execution_id": "9d3f7b2a-4c8e-4a1b-9f6d-8e5c3b2a1d0f",
    "status": "accepted"
  },
  "id": 1
}
```
---

## Conclusion

Lobby simplifies blockchain transaction signing and submission by abstracting away the complex mechanics of nonce management, key custody, and on-chain validation. With its low-latency actor-based architecture and automatic failure recovery, Lobby enables developers to focus on building applications rather than managing transaction infrastructure.

**Key Takeaways:**

- **Fire-and-forget submission** — Receive `execution_id` immediately, poll for status asynchronously
- **Automatic nonce sequencing** — No race conditions, no manual nonce tracking
- **Built-in retry logic** — Transient failures (RPC timeouts, DB hiccups) are handled transparently
- **Multi-chain support** — Single API for Ethereum, Polygon, Arbitrum, and testnets
- **Production-ready architecture** — Actor sharding, semaphore backpressure, database persistence

**Next Steps:**

1. Obtain an API key from your Lobby operator
2. Integrate the client library for your language (Python/TypeScript/Rust examples provided)
3. Test on Hoodi testnet before deploying to production
4. Monitor pipeline metrics (latency, error rates, nonce gaps)
5. Set up alerts for critical failures (broadcast errors, validator timeouts)

**Resources:**

- **GitHub Repository:** [github.com/romanticNomad/Lobby](https://github.com/romanticNomad/Lobby)
- **Issue Tracker:** [github.com/romanticNomad/Lobby/issues](https://github.com/romanticNomad/Lobby/issues)
- **License:** Apache 2.0

---

*Built with Rust, Tokio, Axum, PostgreSQL, and Redis.*  
*Designed for developers who need reliable, low-latency blockchain transaction infrastructure.*

---

> ⚠️ **Reminder:** Lobby is currently in prototype phase. APIs and features may change. Always refer to the [GitHub repository](https://github.com/romanticNomad/Lobby) for the most up-to-date documentation and release notes.
