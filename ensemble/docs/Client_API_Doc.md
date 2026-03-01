# Lobby — Developer Guide: Sending Requests to the Server

> **Lobby** is a low-latency, high-concurrency blockchain transaction signing service.  
> This guide is for developers who want to send transaction requests to a running Lobby instance.  
> **Lobby** is still in the development phase; some features mentioned here may not work as intended.

---

## Table of Contents

1. [Prerequisites](#1-prerequisites)
2. [Authentication](#2-authentication)
3. [Base URL & Endpoints](#3-base-url--endpoints)
4. [Request Format — EIP-1193 JSON-RPC](#4-request-format--eip-1193-json-rpc)
5. [Transaction Fields Reference](#5-transaction-fields-reference)
6. [Response Format](#6-response-format)
7. [Error Codes Reference](#7-error-codes-reference)
8. [Tracking Your Transaction — Status Polling](#8-tracking-your-transaction--status-polling)
9. [Code Examples — Transaction Submission](#9-code-examples--transaction-submission)
10. [Code Examples — Status Polling](#10-code-examples--status-polling)
11. [Supported Chains](#11-supported-chains)
12. [Running Lobby Locally](#12-running-lobby-locally)
13. [FAQ](#13-faq)

---

## 1. Prerequisites

Before sending requests to Lobby you need the following:

- A **running Lobby instance** (see [Section 12](#12-running-lobby-locally) if you are running it locally).
- An **API key** issued by the Lobby operator (see [Section 2](#2-authentication)).
- The **Ethereum address** your API key is bound to. Lobby is a custodial service — it holds the private key for your address. All transactions you submit must originate from this address.
- A **chain ID** for the network you want to transact on (see [Section 11](#11-supported-chains)).

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
lobby_live_<random_string>:<client_id>:<from_address>
```

**Example:**
```
lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92
```

---

## 3. Base URL & Endpoints

### Base URLs

| Environment | Base URL |
|---|---|
| Local development | `http://localhost:3000` |
| Production | `https://<your-lobby-domain>` |

### Available Endpoints

| Endpoint | Method | Description |
|---|---|---|
| `/v1/transactions` | `POST` | Submit a new transaction |
| `/status/{execution_id}` | `GET` | Query transaction status |

> **Note:** The status endpoint does **not** use the `/v1` prefix. It is version-independent.

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
      "maxPriorityFeePerGas":  "<hex_encoded_max_priority_fee_per_gas>"
    }
  ],
  "id": 1
}
```

### Required Fields

The following fields are **required**:

| Field | Required | Description |
|---|---|---|
| `jsonrpc` | ✅ Yes | Must be exactly `"2.0"` |
| `method` | ✅ Yes | Must be exactly `"eth_sendTransaction"` |
| `params` | ✅ Yes | Array containing exactly one transaction object |
| `from` | ✅ Yes | Must match the address bound to your API key |
| `chainId` | ✅ Yes | Hex-encoded chain ID (e.g. `"0x1"` for Ethereum mainnet) |
| `gas` | ✅ Yes | Hex-encoded gas limit |
| `maxFeePerGas` | ✅ Yes | Hex-encoded maximum fee per gas (EIP-1559) |
| `maxPriorityFeePerGas` | ✅ Yes | Hex-encoded priority fee per gas (EIP-1559) |
| `id` | ✅ Yes | Any integer or string. Echoed back in the response |

### Optional Fields

| Field | Default Behavior |
|---|---|
| `to` | If omitted or `null`, Lobby treats this as a contract creation transaction |
| `value` | Defaults to `"0x0"` (no ETH transfer) |
| `data` | Defaults to `"0x"` (empty calldata) |

> **Gas Estimation:** Lobby does **not** currently support automatic gas estimation. All gas-related fields (`gas`, `maxFeePerGas`, `maxPriorityFeePerGas`) are **required**. Gas estimation is planned for a future release.

---

## 5. Transaction Fields Reference

All numeric values are **hex-encoded strings** with a `0x` prefix, following the EIP-1193 standard.

| Field | Type | Example | Notes |
|---|---|---|---|
| `from` | `address` | `"0x742d35Cc...f0bEb"` | Checksummed or lowercase both accepted |
| `to` | `address \| null` | `"0x5aAeb6...BeAed"` | Omit or set to `null` for contract deployments |
| `value` | `hex uint256` | `"0xde0b6b3a7640000"` | Amount in **wei** (1 ETH = `1e18` wei) |
| `data` | `hex bytes` | `"0xa9059cbb000..."` | ABI-encoded function call |
| `chainId` | `hex uint64` | `"0x1"` | See [supported chains](#11-supported-chains) |
| `gas` | `hex uint256` | `"0x5208"` | Gas limit (21000 = simple ETH transfer) |
| `maxFeePerGas` | `hex uint256` | `"0xba43b7400"` | Max total fee per gas unit in wei |
| `maxPriorityFeePerGas` | `hex uint256` | `"0x77359400"` | Miner tip per gas unit in wei |

### Converting Values to Hex

**Python:**
```python
hex(1_000_000_000_000_000_000)  # 1 ETH in wei → "0xde0b6b3a7640000"
hex(21000)                       # Simple transfer gas → "0x5208"
hex(50_000_000_000)              # 50 gwei maxFeePerGas → "0xba43b7400"
```

**JavaScript:**
```javascript
(1_000_000_000_000_000_000n).toString(16)  // "de0b6b3a7640000"
"0x" + (1_000_000_000_000_000_000n).toString(16)  // "0xde0b6b3a7640000"
```

**Rust:**
```rust
format!("0x{:x}", 1_000_000_000_000_000_000u128)  // "0xde0b6b3a7640000"
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
permit_acquired → accepted → nonce_reserved → signed → broadcasted → confirmed
                                                              ↓
                                                          failed
```

Use the `execution_id` to query status (see [Section 8](#8-tracking-your-transaction--status-polling)).

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
| `500 Internal Server Error` | `-32603` | Internal Lobby error | Retry with exponential backoff |

### Common Errors and Fixes

**`"From address does not match authenticated account"`**
```
Your request:  "from": "0xABCD..."
Your API key:  bound to "0x1234..."
Fix:           Use the address your API key is bound to
```

**`"max_priority_fee_per_gas cannot exceed max_fee_per_gas"`**
```
Ensure: maxPriorityFeePerGas <= maxFeePerGas
```

**`"Missing required field: gas"`**
```
Gas estimation is not yet supported.
You must provide: gas, maxFeePerGas, maxPriorityFeePerGas
```

---

## 8. Tracking Your Transaction — Status Polling

After receiving a `202 Accepted` response, your transaction enters Lobby's internal processing pipeline. Use the `execution_id` to track its progress.

### Status Polling Endpoint

```
GET /status/{execution_id}
```

**Headers:**
```http
Authorization: Bearer <your_api_key>
```

> **Note:** This endpoint does **not** use the `/v1` prefix. It is version-independent.

### Transaction Lifecycle States

| Status | Description | Terminal? |
|---|---|---|
| `permit_acquired` | Pipeline concurrency permit acquired | No |
| `accepted` | Request validated and persisted by RelayHost | No |
| `nonce_reserved` | Nonce successfully assigned, awaiting signing | No |
| `signed` | Transaction signed, awaiting broadcast | No |
| `broadcasted` | Transaction submitted to blockchain RPC node | No |
| `confirmed` | Transaction included on-chain with ≥3 confirmations | ✅ Yes |
| `failed` | Pipeline failed at a specific stage | ✅ Yes |

### Response Format

**In-Progress Transaction:**
```json
{
  "execution_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "nonce_reserved"
}
```

**Broadcasted (Waiting for Confirmation):**
```json
{
  "execution_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "broadcasted",
  "tx_hash": "0xabc123def456..."
}
```

**Confirmed (Success):**
```json
{
  "execution_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "confirmed",
  "tx_hash": "0xabc123def456..."
}
```

**Failed:**
```json
{
  "execution_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "failed",
  "stage": "validator",
  "reason": "timeout after 300s waiting for tx inclusion"
}
```

### Polling Strategy

**Recommended approach:**
- **Poll interval:** Every 3-5 seconds
- **Typical completion time:** 
  - Internal processing (nonce assignment, signing): < 1 second
  - Blockchain broadcast and confirmation: 30-300 seconds (depends on network congestion)
- **Maximum wait time:** 5 minutes (Lobby's validator timeout)
- **Backoff strategy:** Use constant interval polling (no exponential backoff needed)

**Best practices:**
1. Poll immediately after receiving `202 Accepted`
2. Continue polling until status is `confirmed` or `failed`
3. Stop polling after 5 minutes if status remains `broadcasted` (likely failed validation)
4. Store the `tx_hash` once available (appears in `broadcasted` and `confirmed` states)

### Status Retention

> **Note:** In the current prototype, status entries remain queryable indefinitely while Lobby is running (in-memory storage). Future versions will persist status to a database with configurable retention policies.

### Idempotency Behavior

Lobby generates a **unique `execution_id`** for every transaction submission. If you query a non-existent `execution_id`, you will receive an error.

If an `execution_id` already exists in Lobby's internal state (unlikely due to UUID v4 collision resistance), querying it will return the current status of that transaction. This is **not an error condition** — you will simply receive the existing state.

> **Important:** You cannot provide your own `execution_id`. Each `POST /v1/transactions` request generates a fresh `execution_id` server-side. For safe retries, **store the returned `execution_id`** and poll its status instead of resubmitting the transaction.

---

## 9. Code Examples — Transaction Submission

### cURL

**Minimal ETH transfer:**
```bash
curl -X POST http://localhost:3000/v1/transactions \
  -H "Authorization: Bearer lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_sendTransaction",
    "params": [{
      "from":                 "0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92",
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
  -H "Authorization: Bearer lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_sendTransaction",
    "params": [{
      "from":                 "0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92",
      "to":                   "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
      "value":                "0x0",
      "data":                 "0xa9059cbb0000000000000000000000005aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed0000000000000000000000000000000000000000000000000000000077359400",
      "chainId":              "0x1",
      "gas":                  "0xd6d8",
      "maxFeePerGas":         "0xba43b7400",
      "maxPriorityFeePerGas": "0x77359400"
    }],
    "id": 1
  }'
```

---

### Python

```python
import requests
import time

LOBBY_URL  = "http://localhost:3000"
API_KEY    = "lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92"
FROM_ADDR  = "0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92"

def submit_transaction(
    to: str,
    value_wei: int,
    data: str = "0x",
    chain_id: int = 1,
    gas: int = 21000,
    max_fee_per_gas: int = 50_000_000_000,
    max_priority_fee_per_gas: int = 2_000_000_000
) -> str:
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
                "from":                 FROM_ADDR,
                "to":                   to,
                "value":                hex(value_wei),
                "data":                 data,
                "chainId":              hex(chain_id),
                "gas":                  hex(gas),
                "maxFeePerGas":         hex(max_fee_per_gas),
                "maxPriorityFeePerGas": hex(max_priority_fee_per_gas),
            }],
            "id": 1,
        },
    )

    response.raise_for_status()
    body = response.json()

    if "error" in body:
        raise Exception(f"Lobby error {body['error']['code']}: {body['error']['message']}")

    execution_id = body["result"]["execution_id"]
    print(f"✓ Transaction accepted. execution_id={execution_id}")
    return execution_id


# Example: send 0.1 ETH to an address
execution_id = submit_transaction(
    to        = "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
    value_wei = 100_000_000_000_000_000,  # 0.1 ETH
    chain_id  = 1,
)
```

---

### TypeScript / JavaScript

```typescript
const LOBBY_URL = "http://localhost:3000";
const API_KEY   = "lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92";
const FROM_ADDR = "0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92";

async function submitTransaction(params: {
  to:                   string;
  value:                bigint;
  data?:                string;
  chainId:              number;
  gas:                  bigint;
  maxFeePerGas:         bigint;
  maxPriorityFeePerGas: bigint;
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
        from:                 FROM_ADDR,
        to:                   params.to,
        value:                `0x${params.value.toString(16)}`,
        data:                 params.data ?? "0x",
        chainId:              `0x${params.chainId.toString(16)}`,
        gas:                  `0x${params.gas.toString(16)}`,
        maxFeePerGas:         `0x${params.maxFeePerGas.toString(16)}`,
        maxPriorityFeePerGas: `0x${params.maxPriorityFeePerGas.toString(16)}`,
      }],
      id: 1,
    }),
  });

  const body = await response.json();

  if (body.error) {
    throw new Error(`Lobby error ${body.error.code}: ${body.error.message}`);
  }

  const { execution_id } = body.result;
  console.log(`✓ Transaction accepted. execution_id=${execution_id}`);
  return execution_id;
}

// Example: send 0.1 ETH
const executionId = await submitTransaction({
  to:                   "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
  value:                100_000_000_000_000_000n, // 0.1 ETH in wei
  chainId:              1,
  gas:                  21000n,
  maxFeePerGas:         50_000_000_000n,
  maxPriorityFeePerGas: 2_000_000_000n,
});
```

---

### Rust

```rust
use reqwest::Client;
use serde::{Deserialize, Serialize};

const LOBBY_URL: &str = "http://localhost:3000";
const API_KEY:   &str = "lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92";
const FROM_ADDR: &str = "0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92";

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
    from:                   String,
    to:                     String,
    value:                  String,
    data:                   String,
    chain_id:               String,
    gas:                    String,
    max_fee_per_gas:        String,
    max_priority_fee_per_gas: String,
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
    gas:       u64,
    max_fee_per_gas: u128,
    max_priority_fee_per_gas: u128,
) -> Result<String, Box<dyn std::error::Error>> {
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method:  "eth_sendTransaction".to_string(),
        params:  vec![TxParams {
            from:                   FROM_ADDR.to_string(),
            to:                     to.to_string(),
            value:                  format!("0x{:x}", value_wei),
            data:                   "0x".to_string(),
            chain_id:               format!("0x{:x}", chain_id),
            gas:                    format!("0x{:x}", gas),
            max_fee_per_gas:        format!("0x{:x}", max_fee_per_gas),
            max_priority_fee_per_gas: format!("0x{:x}", max_priority_fee_per_gas),
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
    println!("✓ Transaction accepted: execution_id={}", result.execution_id);
    Ok(result.execution_id)
}
```

---

## 10. Code Examples — Status Polling

Once you have an `execution_id`, poll the status endpoint to track your transaction's progress through Lobby's pipeline.

### Python

```python
import requests
import time
from typing import Dict, Any, Optional

LOBBY_URL = "http://localhost:3000"
API_KEY   = "lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92"

def get_transaction_status(execution_id: str) -> Dict[str, Any]:
    """
    Query the status of a transaction.
    Returns the full status response as a dictionary.
    """
    response = requests.get(
        f"{LOBBY_URL}/status/{execution_id}",
        headers={"Authorization": f"Bearer {API_KEY}"},
    )
    
    response.raise_for_status()
    return response.json()


def poll_until_complete(
    execution_id: str,
    poll_interval: float = 3.0,
    timeout: float = 300.0
) -> Dict[str, Any]:
    """
    Poll transaction status until it reaches a terminal state (confirmed or failed).
    
    Args:
        execution_id: The UUID returned by submit_transaction()
        poll_interval: Seconds to wait between polls (default: 3s)
        timeout: Maximum time to wait in seconds (default: 300s)
    
    Returns:
        Final status dictionary
    
    Raises:
        TimeoutError: If transaction doesn't complete within timeout period
    """
    start_time = time.time()
    
    while True:
        elapsed = time.time() - start_time
        if elapsed > timeout:
            raise TimeoutError(
                f"Transaction {execution_id} did not complete within {timeout}s"
            )
        
        status = get_transaction_status(execution_id)
        state = status["status"]
        
        print(f"[{elapsed:.1f}s] Status: {state}", end="")
        
        # Print tx_hash if available
        if "tx_hash" in status:
            print(f" | tx_hash: {status['tx_hash']}", end="")
        
        print()  # newline
        
        # Check for terminal states
        if state == "confirmed":
            print(f"✓ Transaction confirmed! tx_hash: {status['tx_hash']}")
            return status
        
        if state == "failed":
            print(f"✗ Transaction failed at stage: {status['stage']}")
            print(f"  Reason: {status['reason']}")
            return status
        
        # Not done yet, wait and retry
        time.sleep(poll_interval)


# Example usage
execution_id = "550e8400-e29b-41d4-a716-446655440000"

try:
    final_status = poll_until_complete(execution_id)
    
    if final_status["status"] == "confirmed":
        print(f"Success! View on Etherscan: https://etherscan.io/tx/{final_status['tx_hash']}")
    else:
        print("Transaction failed. Check logs for details.")
        
except TimeoutError as e:
    print(f"Error: {e}")
    print("Transaction may still be processing. Check status later.")
```

**Example output:**
```
[0.1s] Status: accepted
[3.2s] Status: nonce_reserved
[6.3s] Status: signed
[9.5s] Status: broadcasted | tx_hash: 0xabc123def456...
[12.6s] Status: broadcasted | tx_hash: 0xabc123def456...
[15.8s] Status: broadcasted | tx_hash: 0xabc123def456...
[18.9s] Status: confirmed | tx_hash: 0xabc123def456...
✓ Transaction confirmed! tx_hash: 0xabc123def456...
Success! View on Etherscan: https://etherscan.io/tx/0xabc123def456...
```

---

### TypeScript / JavaScript

```typescript
const LOBBY_URL = "http://localhost:3000";
const API_KEY   = "lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92";

interface StatusResponse {
  execution_id: string;
  status: string;
  tx_hash?: string;
  stage?: string;
  reason?: string;
}

async function getTransactionStatus(executionId: string): Promise<StatusResponse> {
  const response = await fetch(`${LOBBY_URL}/status/${executionId}`, {
    headers: { "Authorization": `Bearer ${API_KEY}` },
  });
  
  if (!response.ok) {
    throw new Error(`Status query failed: ${response.status} ${response.statusText}`);
  }
  
  return response.json();
}

async function pollUntilComplete(
  executionId: string,
  pollInterval: number = 3000,
  timeout: number = 300000
): Promise<StatusResponse> {
  const startTime = Date.now();
  
  while (true) {
    const elapsed = Date.now() - startTime;
    
    if (elapsed > timeout) {
      throw new Error(
        `Transaction ${executionId} did not complete within ${timeout / 1000}s`
      );
    }
    
    const status = await getTransactionStatus(executionId);
    
    let logLine = `[${(elapsed / 1000).toFixed(1)}s] Status: ${status.status}`;
    if (status.tx_hash) {
      logLine += ` | tx_hash: ${status.tx_hash}`;
    }
    console.log(logLine);
    
    // Check for terminal states
    if (status.status === "confirmed") {
      console.log(`✓ Transaction confirmed! tx_hash: ${status.tx_hash}`);
      return status;
    }
    
    if (status.status === "failed") {
      console.log(`✗ Transaction failed at stage: ${status.stage}`);
      console.log(`  Reason: ${status.reason}`);
      return status;
    }
    
    // Wait before next poll
    await new Promise(resolve => setTimeout(resolve, pollInterval));
  }
}

// Example usage
const executionId = "550e8400-e29b-41d4-a716-446655440000";

try {
  const finalStatus = await pollUntilComplete(executionId);
  
  if (finalStatus.status === "confirmed") {
    console.log(`Success! View on Etherscan: https://etherscan.io/tx/${finalStatus.tx_hash}`);
  } else {
    console.log("Transaction failed. Check logs for details.");
  }
} catch (error) {
  console.error(`Error: ${error.message}`);
  console.log("Transaction may still be processing. Check status later.");
}
```

---

### Rust

```rust
use reqwest::Client;
use serde::Deserialize;
use std::time::{Duration, Instant};
use tokio::time::sleep;

const LOBBY_URL: &str = "http://localhost:3000";
const API_KEY:   &str = "lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92";

#[derive(Debug, Deserialize)]
struct StatusResponse {
    execution_id: String,
    status: String,
    tx_hash: Option<String>,
    stage: Option<String>,
    reason: Option<String>,
}

async fn get_transaction_status(
    client: &Client,
    execution_id: &str,
) -> Result<StatusResponse, Box<dyn std::error::Error>> {
    let response = client
        .get(format!("{}/status/{}", LOBBY_URL, execution_id))
        .header("Authorization", format!("Bearer {}", API_KEY))
        .send()
        .await?;
    
    if !response.status().is_success() {
        return Err(format!("Status query failed: {}", response.status()).into());
    }
    
    Ok(response.json().await?)
}

async fn poll_until_complete(
    client: &Client,
    execution_id: &str,
    poll_interval: Duration,
    timeout: Duration,
) -> Result<StatusResponse, Box<dyn std::error::Error>> {
    let start_time = Instant::now();
    
    loop {
        let elapsed = start_time.elapsed();
        
        if elapsed > timeout {
            return Err(format!(
                "Transaction {} did not complete within {:?}",
                execution_id, timeout
            ).into());
        }
        
        let status = get_transaction_status(client, execution_id).await?;
        
        let mut log_line = format!(
            "[{:.1}s] Status: {}",
            elapsed.as_secs_f64(),
            status.status
        );
        
        if let Some(ref tx_hash) = status.tx_hash {
            log_line.push_str(&format!(" | tx_hash: {}", tx_hash));
        }
        
        println!("{}", log_line);
        
        // Check for terminal states
        match status.status.as_str() {
            "confirmed" => {
                println!(
                    "✓ Transaction confirmed! tx_hash: {}",
                    status.tx_hash.as_ref().unwrap()
                );
                return Ok(status);
            }
            "failed" => {
                println!(
                    "✗ Transaction failed at stage: {}",
                    status.stage.as_ref().unwrap()
                );
                println!(
                    "  Reason: {}",
                    status.reason.as_ref().unwrap()
                );
                return Ok(status);
            }
            _ => {}
        }
        
        // Wait before next poll
        sleep(poll_interval).await;
    }
}

// Example usage
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new();
    let execution_id = "550e8400-e29b-41d4-a716-446655440000";
    
    match poll_until_complete(
        &client,
        execution_id,
        Duration::from_secs(3),
        Duration::from_secs(300),
    ).await {
        Ok(final_status) => {
            if final_status.status == "confirmed" {
                println!(
                    "Success! View on Etherscan: https://etherscan.io/tx/{}",
                    final_status.tx_hash.unwrap()
                );
            } else {
                println!("Transaction failed. Check logs for details.");
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            println!("Transaction may still be processing. Check status later.");
        }
    }
    
    Ok(())
}
```

---

## 11. Supported Chains

Lobby currently supports the following blockchain networks:

| Chain | Chain ID (decimal) | Chain ID (hex) |
|---|---|---|
| Ethereum Mainnet | `1` | `0x1` |
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

> **Note:** Chain support is determined by the RPC providers configured in the Lobby instance. Contact your Lobby operator to add support for additional chains.

---

## 12. Running Lobby Locally

To run Lobby on your local machine for development purposes:

### Prerequisites

- **Rust** (stable, latest) — install via [rustup.rs](https://rustup.rs)
- **PostgreSQL** — running locally or via Docker
- **RPC provider API keys** (Alchemy, Infura, or similar) for each chain you want to support

---

### Step 1: Clone and Build

```bash
git clone https://github.com/romanticNomad/Lobby.git
cd lobby
cargo build --release
```

---

### Step 2: Set Up the Database

**Using Docker (recommended):**
```bash
docker run -d \
  --name lobby-postgres \
  -e POSTGRES_PASSWORD=password \
  -e POSTGRES_DB=lobby \
  -p 5432:5432 \
  postgres:15
```

**Run migrations:**
```bash
DATABASE_URL=postgresql://postgres:password@localhost/lobby \
cargo run --bin migrate
```

---

### Step 3: Configure Test Accounts

Lobby loads private keys from a `test_keys.json` file in the project root. Create this file with your test accounts:

```json
{
  "account1": {
    "pvt_key": "0xe74176dc8bcf2e5e6500a8f117a665ed44bcf448206c0cd23cb1228e61a2729c",
    "pub_key": "0x045ae5fcf5e6e9fafbec1155a363467bdea7eeb453bf1e68944b873f81871344dc38209d99d4af74dcbcba7da410353cbd45241cc9e91b863c170a8b1bda7e78a3",
    "address": "0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92"
  },
  "account2": {
    "pvt_key": "0x25bf69fff0829179e3b4fcc1691c766b3126d78b8d80fe48f1322f074404ac5f",
    "pub_key": "0x040d382e58ebef374d9fe943891c11cbf0164ff9204370aaf1a6e8d8100773a8bfaf5342f2edee33263565c078c1babbf7ceed4c6dea9456dd72534e6803e105fa",
    "address": "0x5b35c4571b935076c853e9c3aab59b60d7b32daf"
  }
}
```

> **Security Warning:** The `test_keys.json` file contains **unencrypted private keys**. This is **only suitable for local testing with testnet accounts**. Never use this approach in production or with real funds.

---

### Step 4: Configure Environment Variables

Create a `.env` file in the project root:

```bash
# Database
DATABASE_URL=postgresql://postgres:password@localhost/lobby

# Server
SERVER_ADDR=0.0.0.0:3000

# Orchestrator Configuration
NONCE_SHARDS=17
SIGN_SHARDS=17
BROADCAST_SHARDS=17
ACTOR_BUFFER_SIZE=64
PIPELINE_CONCURRENCY=17
PIPELINE_SEMAPHORE_TIMEOUT_MS=5000

# API Keys
# Format: LOBBY_API_KEY_<N>=<api_token>:<client_id>:<from_address>
LOBBY_API_KEY_1=lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92

# RPC Providers
# Format: RPC_PROVIDER_<chain_id>=<https_url>
RPC_PROVIDER_1=https://eth-mainnet.g.alchemy.com/v2/YOUR_ALCHEMY_KEY
RPC_PROVIDER_11155111=https://eth-sepolia.g.alchemy.com/v2/YOUR_ALCHEMY_KEY
RPC_PROVIDER_137=https://polygon-mainnet.g.alchemy.com/v2/YOUR_ALCHEMY_KEY
RPC_PROVIDER_42161=https://arb-mainnet.g.alchemy.com/v2/YOUR_ALCHEMY_KEY

# Logging
RUST_LOG=info
```

**Important notes:**
- The `from_address` in your API key must match one of the addresses in `test_keys.json`
- Each `RPC_PROVIDER_<chain_id>` must correspond to a supported chain
- Replace `YOUR_ALCHEMY_KEY` with your actual Alchemy API key (or use Infura/other providers)

---

### Step 5: Start the Server

```bash
source .env
cargo run --release
```

You should see:
```
INFO lobby: lobby starting
INFO lobby: database connection estabilished
INFO sqlx::postgres::notice: relation "_sqlx_migrations" already exists, skipping
NFO lobby: database migrations applied
INFO lobby: api keys loaded count=1
INFO lobby: RPC provider registry initialized chains=[ChainId(1)] count=1
INFO cortex: spawning cortex actor pools nonce_shards=17 sign_shards=17 broadcast_shards=17 pipeline=17
NFO cortex: orchestrator ready
NFO lobby: cortex ready
NFO actors::relayhost::engine: RelayHostEngine started
NFO actors::validator::engine: validator engine started
NFO lobby: lobby listening server_addr=0.0.0.0:3000
```

---

### Step 6: Verify the Server is Running

```bash
curl -X POST http://localhost:3000/v1/transactions \
  -H "Authorization: Bearer lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method":  "eth_sendTransaction",
    "params": [{
      "from":                 "0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92",
      "to":                   "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
      "value":                "0xde0b6b3a7640000",
      "chainId":              "0xaa36a7",
      "gas":                  "0x5208",
      "maxFeePerGas":         "0xba43b7400",
      "maxPriorityFeePerGas": "0x77359400"
    }],
    "id": 1
  }'
```

Expected response:
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

Then query the status:
```bash
curl -H "Authorization: Bearer lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92" \
  http://localhost:3000/status/550e8400-e29b-41d4-a716-446655440000
```

---

## 13. FAQ

**Q: Do I need to provide a nonce?**  
No. Nonce management is one of Lobby's core responsibilities. Never include a `nonce` field — it is assigned internally by the Nonce actor.

**Q: Do I need to sign the transaction myself?**  
No. Lobby is a custodial signing service. It holds the private key for your bound address and signs all transactions internally via its Sign actor.

**Q: What happens if the transaction fails on-chain?**  
Lobby's Validator actor detects on-chain failures (e.g., transaction reverted, not included after 5 minutes). The status will be updated to `failed` with details in the `stage` and `reason` fields. The nonce is automatically released for reuse.

**Q: Can one API key control multiple addresses?**  
No. Each API key is bound to exactly one `from_address`. To sign from multiple addresses, obtain separate API keys for each.

**Q: Is the `202 Accepted` response a guarantee my transaction will be mined?**  
No. `202 Accepted` means Lobby has validated and accepted your transaction into its pipeline. Network conditions, insufficient gas, RPC issues, or on-chain reverts may still prevent the transaction from being confirmed. Always poll the status endpoint to verify the final outcome.

**Q: What encoding should I use for addresses?**  
Both checksummed (EIP-55) and lowercase hex addresses are accepted. Examples:
- Checksummed: `0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed`
- Lowercase: `0x5aaeb6053f3e94c9b9a09f33669435e7ef1beaed`

Both are valid.

**Q: Can I send a contract deployment transaction?**  
Yes. Omit the `to` field entirely (or set it to `null`). Provide your deployment bytecode in the `data` field. Lobby will treat this as a contract creation transaction.

**Q: How do I estimate gas for my transaction?**  
Lobby does not currently support automatic gas estimation. You must provide `gas`, `maxFeePerGas`, and `maxPriorityFeePerGas` values. Use tools like:
- **Alchemy/Infura:** Call `eth_estimateGas` and `eth_feeHistory` via their APIs
- **ethers.js:** `provider.estimateGas(tx)` and `provider.getFeeData()`
- **web3.py:** `web3.eth.estimate_gas(tx)` and `web3.eth.gas_price`

Gas estimation will be added in a future release.

**Q: What happens if I lose my `execution_id`?**  
If you lose the `execution_id` returned by Lobby (e.g., client crash before storing it), you cannot query the status of that transaction. This is why it's critical to **persist the `execution_id` immediately** after receiving a `202 Accepted` response.

**Q: Can I cancel or replace a transaction?**  
Not currently. Once a transaction is accepted and enters the pipeline, Lobby will process it to completion (either `confirmed` or `failed`). Transaction replacement/cancellation is planned for a future release.

**Q: How long does status remain queryable?**  
In the current prototype, status entries remain in memory indefinitely while Lobby is running. If Lobby restarts, the in-memory status registry is lost. Future versions will persist status to a database with configurable retention policies.

---

## Contributing

Lobby is an open-source project. Contributions, issues, and discussions are welcome.

- **Repository:** [github.com/romanticNomad/Lobby](https://github.com/romanticNomad/Lobby)
- **Issues:** [GitHub Issues](https://github.com/romanticNomad/Lobby/issues)
- **License:** Apache-2.0

---

*Built with Rust, Tokio, Axum, and PostgreSQL.*
