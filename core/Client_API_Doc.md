# Lobby — Developer Guide: Sending Requests to the Server

> **Lobby** is a low-latency, high-concurrency blockchain transaction signing service.  
> This guide is for developers who want to send transaction requests to a running Lobby instance.

---

## Table of Contents

1. [Prerequisites](#1-prerequisites)
2. [Authentication](#2-authentication)
3. [Base URL](#3-base-url)
4. [Request Format — EIP-1193 JSON-RPC](#4-request-format--eip-1193-json-rpc)
5. [Transaction Fields Reference](#5-transaction-fields-reference)
6. [Response Format](#6-response-format)
7. [Error Codes Reference](#7-error-codes-reference)
8. [Code Examples](#8-code-examples)
9. [Tracking Your Transaction](#9-tracking-your-transaction)
10. [Supported Chains](#10-supported-chains)
11. [Running Lobby Locally](#11-running-lobby-locally)
12. [FAQ](#12-faq)

---

## 1. Prerequisites

Before sending requests to Lobby you need the following:

- A **running Lobby instance** (see [Section 11](#11-running-lobby-locally) if you are running it locally).
- An **API key** issued by the Lobby operator (see [Section 2](#2-authentication)).
- The **Ethereum address** your API key is bound to. Lobby is a custodial service — it holds the private key for your address. All transactions you submit must originate from this address.
- A **chain ID** for the network you want to transact on (see [Section 10](#10-supported-chains)).

---

## 2. Authentication

Lobby uses **Bearer Token authentication**. Every request must include an `Authorization` header with your API key.

```http
Authorization: Bearer <your_api_key>
```

### Obtaining an API Key

API keys are provisioned by the Lobby operator (the person running the server). Each key is bound to:

| Property | Description |
|---|---|
| `client_id` | A UUID identifying your account |
| `from_address` | The Ethereum address Lobby will sign transactions from on your behalf |

> **Important:** You cannot submit transactions from an address other than the one bound to your API key. Attempts to do so will return a `403 Forbidden` error.

### Key Format

Lobby API keys follow this format:

```
lobby_live_<random_string>
```

Example: `lobby_live_abc123xyz789`

---

## 3. Base URL

| Environment | Base URL |
|---|---|
| Local development | `http://localhost:3000` |
| Production | `https://<your-lobby-domain>` |

All endpoints are prefixed with `/v1`.

---

## 4. Request Format — EIP-1193 JSON-RPC

Lobby accepts standard **EIP-1193 JSON-RPC** formatted requests on the transaction submission endpoint.

### Endpoint

```
POST /v1/transactions
```

### Headers

```http
Content-Type: application/json
Authorization: Bearer <your_api_key>
```

### Body Schema

```json
{
  "jsonrpc": "2.0",
  "method": "eth_sendTransaction",
  "params": [
    {
      "from":                  "<your_bound_address>",
      "to":                    "<recipient_or_contract_address>",
      "value":                 "<hex_encoded_wei_amount>",
      "data":                  "<hex_encoded_calldata>",
      "chainId":               "<hex_encoded_chain_id>",
      "gas":                   "<hex_encoded_gas_limit>",
      "maxFeePerGas":          "<hex_encoded_max_fee_per_gas>",
      "maxPriorityFeePerGas":  "<hex_encoded_max_priority_fee_per_gas>",
      "accessList":            []
    }
  ],
  "id": 1
}
```

### Minimal Required Fields

The following fields are **required**. All others are optional — Lobby will estimate them if absent.

| Field | Required | Description |
|---|---|---|
| `jsonrpc` | ✅ Yes | Must be exactly `"2.0"` |
| `method` | ✅ Yes | Must be exactly `"eth_sendTransaction"` |
| `params` | ✅ Yes | Array containing exactly one transaction object |
| `from` | ✅ Yes | Must match the address bound to your API key |
| `chainId` | ✅ Yes | Hex-encoded chain ID (e.g. `"0x1"` for Ethereum mainnet) |
| `id` | ✅ Yes | Any integer or string. Echoed back in the response |

### Optional Fields

| Field | Default Behaviour |
|---|---|
| `to` | If omitted, Lobby treats this as a contract creation transaction |
| `value` | Defaults to `0x0` (no ETH transfer) |
| `data` | Defaults to `0x` (empty calldata) |
| `gas` | Lobby calls `eth_estimateGas` and uses the result |
| `maxFeePerGas` | Lobby fetches the current base fee and adds a buffer |
| `maxPriorityFeePerGas` | Lobby uses a reasonable tip estimate |
| `accessList` | Defaults to `[]` |

> **Gas Reconciliation Rule:** If you provide `gas`, `maxFeePerGas`, or `maxPriorityFeePerGas`, Lobby will estimate the values independently and use `min(your_value, lobby_estimate)`. This prevents overpaying while still respecting your upper bounds.

---

## 5. Transaction Fields Reference

All numeric values are **hex-encoded strings** with a `0x` prefix, following the EIP-1193 standard.

| Field | Type | Example | Notes |
|---|---|---|---|
| `from` | `address` | `"0x742d35Cc...f0bEb"` | Checksummed or lowercase both accepted |
| `to` | `address \| null` | `"0x5aAeb6...BeAed"` | Omit for contract deployments |
| `value` | `hex uint256` | `"0xde0b6b3a7640000"` | Amount in **wei** (1 ETH = `1e18` wei) |
| `data` | `hex bytes` | `"0xa9059cbb000..."` | ABI-encoded function call |
| `chainId` | `hex uint64` | `"0x1"` | See supported chains below |
| `gas` | `hex uint256` | `"0x5208"` | Gas limit (21000 = simple ETH transfer) |
| `maxFeePerGas` | `hex uint256` | `"0xba43b7400"` | Max total fee per gas unit in wei |
| `maxPriorityFeePerGas` | `hex uint256` | `"0x77359400"` | Miner tip per gas unit in wei |
| `accessList` | `array` | `[]` | EIP-2930 access list entries |

### Converting Values to Hex

```python
# Python
hex(1_000_000_000_000_000_000)  # 1 ETH in wei → "0xde0b6b3a7640000"
hex(21000)                       # Simple transfer gas → "0x5208"
hex(50_000_000_000)              # 50 gwei maxFeePerGas → "0xba43b7400"
```

```javascript
// JavaScript
(1_000_000_000_000_000_000n).toString(16)  // "de0b6b3a7640000" → prepend "0x"
```

---

## 6. Response Format

### Success — `202 Accepted`

Lobby returns `202 Accepted` immediately after the transaction has been validated and accepted into the processing pipeline. **This does not mean the transaction has been mined yet.**

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

| Field | Description |
|---|---|
| `execution_id` | A UUID v4 that uniquely identifies this transaction intent across the entire Lobby pipeline. **Save this.** You will use it to track the transaction's lifecycle. |
| `status` | Always `"accepted"` on success |

### What Happens After `202 Accepted`?

Once accepted, Lobby processes your transaction through an internal pipeline:

```
Accepted → Nonce Assigned → Signed → Broadcast → Mined (confirmed on-chain)
```

Use the `execution_id` to query status (see [Section 9](#9-tracking-your-transaction)).

---

## 7. Error Codes Reference

All errors follow the **JSON-RPC 2.0 error format**:

```json
{
  "jsonrpc": "2.0",
  "error": {
    "code": -32602,
    "message": "Human-readable description",
    "data": { "field": "extra context if available" }
  },
  "id": 1
}
```

### HTTP Status → JSON-RPC Code Mapping

| HTTP Status | JSON-RPC Code | Meaning | Action |
|---|---|---|---|
| `400 Bad Request` | `-32600` | Invalid JSON-RPC version | Set `jsonrpc` to exactly `"2.0"` |
| `400 Bad Request` | `-32601` | Unsupported method | Use `"eth_sendTransaction"` |
| `400 Bad Request` | `-32602` | Invalid or missing params | Check field types and required fields |
| `401 Unauthorized` | `-32000` | Missing or invalid API key | Check your `Authorization` header |
| `403 Forbidden` | `-32000` | `from` address mismatch | Use the address bound to your API key |
| `409 Conflict` | `-32000` | Duplicate `execution_id` | This transaction was already submitted |
| `500 Internal Server Error` | `-32603` | Internal Lobby error | Retry with exponential backoff |

### Common Errors and Fixes

**`"From address does not match authenticated account"`**
```
Your request:  "from": "0xABCD..."
Your API key:  bound to "0x1234..."
Fix:           Use the address your API key is bound to
```

**`"gas_limit exceeds maximum allowed"`**
```
You provided:  "gas": "0x2FAF080"   (50,000,000 gas)
Lobby maximum: 30,000,000 gas
Fix:           Lower your gas estimate or omit the field
```

**`"max_priority_fee_per_gas cannot exceed max_fee_per_gas"`**
```
Ensure: maxPriorityFeePerGas <= maxFeePerGas
```

**`"Duplicate execution_id"`**
```
This execution_id was already submitted and is being processed.
This is NOT an error — your transaction is already in the pipeline.
```

---

## 8. Code Examples

### cURL

**Minimal ETH transfer:**
```bash
curl -X POST http://localhost:3000/v1/transactions \
  -H "Authorization: Bearer lobby_live_abc123xyz" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_sendTransaction",
    "params": [{
      "from":    "0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb",
      "to":      "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
      "value":   "0xde0b6b3a7640000",
      "chainId": "0x1"
    }],
    "id": 1
  }'
```

**With explicit gas parameters:**
```bash
curl -X POST http://localhost:3000/v1/transactions \
  -H "Authorization: Bearer lobby_live_abc123xyz" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_sendTransaction",
    "params": [{
      "from":                 "0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb",
      "to":                   "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
      "value":                "0xde0b6b3a7640000",
      "chainId":              "0x1",
      "gas":                  "0x5208",
      "maxFeePerGas":         "0xba43b7400",
      "maxPriorityFeePerGas": "0x77359400"
    }],
    "id": 1
  }'
```

**ERC-20 token transfer (with calldata):**
```bash
curl -X POST http://localhost:3000/v1/transactions \
  -H "Authorization: Bearer lobby_live_abc123xyz" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_sendTransaction",
    "params": [{
      "from":    "0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb",
      "to":      "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
      "value":   "0x0",
      "data":    "0xa9059cbb0000000000000000000000005aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed0000000000000000000000000000000000000000000000000000000077359400",
      "chainId": "0x1"
    }],
    "id": 1
  }'
```

---

### Python

```python
import requests

LOBBY_URL  = "http://localhost:3000"
API_KEY    = "lobby_live_abc123xyz"
FROM_ADDR  = "0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb"

def submit_transaction(to: str, value_wei: int, data: str = "0x", chain_id: int = 1) -> str:
    """
    Submit a transaction to Lobby.
    Returns the execution_id on success.
    Raises an exception on failure.
    """
    response = requests.post(
        f"{LOBBY_URL}/v1/transactions",
        headers={
            "Authorization": f"Bearer {API_KEY}",
            "Content-Type":  "application/json",
        },
        json={
            "jsonrpc": "2.0",
            "method":  "eth_sendTransaction",
            "params": [{
                "from":    FROM_ADDR,
                "to":      to,
                "value":   hex(value_wei),
                "data":    data,
                "chainId": hex(chain_id),
            }],
            "id": 1,
        },
    )

    if response.status_code == 409:
        # Duplicate — already in pipeline, not a real error
        body = response.json()
        print(f"Already submitted: {body['error']['message']}")
        return None

    response.raise_for_status()
    body = response.json()

    if "error" in body:
        raise Exception(f"Lobby error {body['error']['code']}: {body['error']['message']}")

    execution_id = body["result"]["execution_id"]
    print(f"Transaction accepted. execution_id={execution_id}")
    return execution_id


# Example: send 1 ETH to an address
execution_id = submit_transaction(
    to        = "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
    value_wei = 10 ** 18,  # 1 ETH
    chain_id  = 1,
)
```

---

### TypeScript / JavaScript

```typescript
const LOBBY_URL = "http://localhost:3000";
const API_KEY   = "lobby_live_abc123xyz";
const FROM_ADDR = "0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb";

async function submitTransaction(params: {
  to:       string;
  value:    bigint;
  data?:    string;
  chainId:  number;
}): Promise<string> {
  const response = await fetch(`${LOBBY_URL}/v1/transactions`, {
    method:  "POST",
    headers: {
      "Authorization": `Bearer ${API_KEY}`,
      "Content-Type":  "application/json",
    },
    body: JSON.stringify({
      jsonrpc: "2.0",
      method:  "eth_sendTransaction",
      params: [{
        from:    FROM_ADDR,
        to:      params.to,
        value:   `0x${params.value.toString(16)}`,
        data:    params.data ?? "0x",
        chainId: `0x${params.chainId.toString(16)}`,
      }],
      id: 1,
    }),
  });

  const body = await response.json();

  if (body.error) {
    throw new Error(`Lobby error ${body.error.code}: ${body.error.message}`);
  }

  const { execution_id } = body.result;
  console.log(`Transaction accepted. execution_id=${execution_id}`);
  return execution_id;
}

// Example: send 0.1 ETH
const executionId = await submitTransaction({
  to:      "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
  value:   100_000_000_000_000_000n, // 0.1 ETH in wei
  chainId: 1,
});
```

---

### Rust

```rust
use reqwest::Client;
use serde::{Deserialize, Serialize};

const LOBBY_URL: &str = "http://localhost:3000";
const API_KEY:   &str = "lobby_live_abc123xyz";
const FROM_ADDR: &str = "0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb";

#[derive(Serialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    method:  String,
    params:  Vec<TxParams>,
    id:      u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TxParams {
    from:     String,
    to:       String,
    value:    String,
    data:     String,
    chain_id: String,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    result: Option<TxResult>,
    error:  Option<JsonRpcError>,
}

#[derive(Deserialize)]
struct TxResult {
    execution_id: String,
}

#[derive(Deserialize)]
struct JsonRpcError {
    code:    i64,
    message: String,
}

async fn submit_transaction(
    client:   &Client,
    to:       &str,
    value_wei: u128,
    chain_id:  u64,
) -> Result<String, Box<dyn std::error::Error>> {
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method:  "eth_sendTransaction".to_string(),
        params:  vec![TxParams {
            from:     FROM_ADDR.to_string(),
            to:       to.to_string(),
            value:    format!("0x{:x}", value_wei),
            data:     "0x".to_string(),
            chain_id: format!("0x{:x}", chain_id),
        }],
        id: 1,
    };

    let response: JsonRpcResponse = client
        .post(format!("{}/v1/transactions", LOBBY_URL))
        .header("Authorization", format!("Bearer {}", API_KEY))
        .json(&request)
        .send()
        .await?
        .json()
        .await?;

    if let Some(err) = response.error {
        return Err(format!("Lobby error {}: {}", err.code, err.message).into());
    }

    let result = response.result.ok_or("Missing result")?;
    println!("Transaction accepted: execution_id={}", result.execution_id);
    Ok(result.execution_id)
}
```

---

## 9. Tracking Your Transaction

After receiving a `202 Accepted` response, your transaction enters Lobby's internal processing pipeline. Use the `execution_id` to track its progress.

### Transaction Lifecycle

```
[accepted] → [nonce_assigned] → [signed] → [broadcast] → [mined]
                                                              ↓
                                                     [finalized] ✅
                                           or
                                                     [failed] ❌  (retry eligible)
```

### Retry / Idempotency

Lobby uses `execution_id` for **idempotency**. If you submit the same transaction twice:

- If Lobby is still processing it: you receive `409 Conflict` with `"Duplicate execution_id"`.
- This is safe and expected retry behaviour.
- If you want to submit a genuinely new transaction, your client must generate a new `execution_id` — but note that Lobby generates `execution_id` internally and returns it to you. Each new `POST /v1/transactions` call always generates a fresh one.

> **Best practice:** Store the `execution_id` returned by Lobby. If you lose it (e.g. client crash), you cannot retry idempotently. Design your client to persist `execution_id` before considering a transaction "sent".

---

## 10. Supported Chains

| Chain | Chain ID (decimal) | Chain ID (hex) |
|---|---|---|
| Ethereum Mainnet | `1` | `0x1` |
| Goerli (testnet) | `5` | `0x5` |
| Sepolia (testnet) | `11155111` | `0xaa36a7` |
| Polygon | `137` | `0x89` |
| Arbitrum One | `42161` | `0xa4b1` |

Submitting a transaction with an unsupported `chainId` will return:
```json
{
  "error": {
    "code": -32602,
    "message": "Unsupported chain_id: 56"
  }
}
```

---

## 11. Running Lobby Locally

To run Lobby on your local machine for development purposes:

### Prerequisites

- **Rust** (stable, latest) — install via [rustup.rs](https://rustup.rs)
- **PostgreSQL** — running locally or via Docker
- **Alchemy or Infura API key** — for RPC provider access

### Step 1: Clone and Build

```bash
git clone https://github.com/your-org/lobby.git
cd lobby
cargo build --release
```

### Step 2: Set Up the Database

```bash
# Using Docker (easiest)
docker run -d \
  --name lobby-postgres \
  -e POSTGRES_PASSWORD=password \
  -e POSTGRES_DB=lobby \
  -p 5432:5432 \
  postgres:15

# Run migrations (from the lobby directory)
DATABASE_URL=postgresql://postgres:password@localhost/lobby \
cargo run --bin migrate
```

### Step 3: Configure Environment Variables

Create a `.env` file in the project root:

```bash
# .env

# Database
DATABASE_URL=postgresql://postgres:password@localhost/lobby

# Server
SERVER_ADDR=0.0.0.0:3000

# API Keys — format: LOBBY_API_KEY_<N>=<api_key>:<client_id>:<from_address>
# client_id must be a valid UUID v4
# from_address must be an Ethereum address (checksummed or lowercase)
LOBBY_API_KEY_1=lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb

# Logging
RUST_LOG=info
```

### Step 4: Start the Server

```bash
source .env
cargo run --release
```

You should see:
```
INFO lobby: Database connection established
INFO lobby: Database migrations applied
INFO lobby: Loaded 1 API keys
INFO lobby: Starting Lobby server on 0.0.0.0:3000
```

### Step 5: Verify the Server is Running

```bash
curl -X POST http://localhost:3000/v1/transactions \
  -H "Authorization: Bearer lobby_live_abc123xyz" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method":  "eth_sendTransaction",
    "params": [{
      "from":    "0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb",
      "to":      "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
      "value":   "0xde0b6b3a7640000",
      "chainId": "0x1"
    }],
    "id": 1
  }'
```

Expected response:
```json
{
  "jsonrpc": "2.0",
  "result": {
    "execution_id": "<uuid>",
    "status": "accepted"
  },
  "id": 1
}
```

---

## 12. FAQ

**Q: Do I need to provide a nonce?**  
No. Nonce management is one of Lobby's core responsibilities. Never include a `nonce` field — it is assigned internally and omitting it is intentional.

**Q: Do I need to sign the transaction myself?**  
No. Lobby is a custodial signing service. It holds the private key for your bound address and signs all transactions internally via its Sign actor.

**Q: What happens if the transaction fails on-chain?**  
Lobby tracks broadcast and confirmation state. Failed transactions are eligible for retry using the same `execution_id` within the idempotency window (5 minutes). After that window, a new request is required.

**Q: Can one API key control multiple addresses?**  
No. Each API key is bound to exactly one `from_address`. To sign from multiple addresses, obtain separate API keys for each.

**Q: Is the `202 Accepted` response a guarantee my transaction will be mined?**  
No. `202 Accepted` means Lobby has validated and accepted your transaction into its pipeline. Network conditions, insufficient gas, or RPC issues may still prevent the transaction from being mined. Use the `execution_id` to track the final outcome.

**Q: What encoding should I use for addresses?**  
Both checksummed (EIP-55) and lowercase hex addresses are accepted. Example: `0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb` and `0x742d35cc6634c0532925a3b844bc9e7595f0beb` are both valid.

**Q: Can I send a contract deployment transaction?**  
Yes. Omit the `to` field entirely. Provide your deployment bytecode in the `data` field.

---

## Contributing

Lobby is an open-source project. Contributions, issues, and discussions are welcome.

- **Issues:** [GitHub Issues](https://github.com/romanticNomad/Lobby/issues)
- **License:** license = "Apache-2.0"

---

*Built with Rust, Tokio, Axum, and PostgreSQL.*