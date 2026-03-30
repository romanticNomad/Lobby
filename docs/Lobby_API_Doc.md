# Lobby Client API Documentation

> * **Prototype Notice:** Lobby is currently in active development. APIs, features, and behaviors described in this document may change in future releases. Please refer to the [GitHub repository](https://github.com/romanticNomad/Lobby) for the latest updates.  
> * **Do Not** use **Lobby** for mainnet transactions.
> * For `test keys` generation the user may use my evm account genration tool **[Locket](https://github.com/romanticNomad/Locket)**, the user can simply get a new account with a `cargo run` command.

**Version:** 0.1.0 (Prototype)  
**Last Updates:** March 26 2026  
**Target Audience:** Contributors, maintainers, and LLMs working with the Lobby codebase

---

## Table of Contents

1. [Authentication & Authorization](#1-authentication--authorization)
2. [API Endpoints Reference](#2-api-endpoints-reference)
3. [Transaction Lifecycle & Status Tracking](#3-transaction-lifecycle--status-tracking)
4. [Error Handling](#4-error-handling)
5. [Client Implementation Examples](#5-client-implementation-examples)
6. [Generating API Keys](#6-generating-api-keys)

---

## 1. Authentication & Authorization

### 1.1 API Key Format

Lobby uses **Bearer token authentication** with structured API keys:

```bash
lobby_live_<random_string>:<client_id>:<from_address>
```

**Example:**

```bash
lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:<test_account_from_address>
```

**Components:**

| Part | Type | Description |
|---|---|---|
| `lobby_live_<random>` | API token | Server-side lookup key |
| `client_id` | UUID v4 | Unique client identifier |
| `from_address` | Ethereum address | Bound address for this key |

---

### 1.2 Authentication Flow

**Request:**

```http
POST /v1/transactions
Authorization: Bearer lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:<test_account_from_address>
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

### 1.3 Address Binding Security

Each API key is **permanently bound** to a single `from_address`. This ensures:

* **No cross-account signing** — Key for address A cannot sign transactions from address B
* **Audit trail** — Every transaction is traceable to a specific client account
* **Key rotation** — Compromised keys can be revoked without affecting other accounts

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

### 1.4 Obtaining API Keys

API keys are provisioned by the Lobby operator. For local development:

1. Add an entry to your `.env` file:

   ```bash
   LOBBY_API_KEY_1=lobby_live_mytoken:550e8400-e29b-41d4-a716-446655440000:0xYOUR_TEST_ADDRESS
   ```

2. Ensure the `from_address` matches an entry in `test_keys.json`:

   ```json
   {
     "account1": {
       "pvt_key": "0x...",
       "pub_key": "0xYOUR_TEST_PUBKEY",
       "address": "0xYOUR_TEST_ADDRESS"
     }
   }
   ```

For production deployments, contact your Lobby operator for secure key provisioning.

---

## 2. API Endpoints Reference

### 2.1 Transaction Submission

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

### 2.2 Status Polling

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

## 3. Transaction Lifecycle & Status Tracking

### 3.1 Status Descriptions

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

### 3.2 Polling Strategy

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

* **Interval:** 3-5 seconds
* **Timeout:** 5 minutes (matches Lobby's validator timeout)
* **Backoff:** Not necessary (constant interval is fine)

**State Retention:**

> **2 layer retention:** Status entries currently persist in-memory and uses Redis persistence with configurable TTL.

---

### 3.3 Terminal State Handling

**Confirmed Transaction:**

```json
{
  "status": "confirmed_on_chain",
  "tx_hash": "0xabc123..."
}
```

**Action:** Transaction succeeded. You can:

* View on block explorer (Etherscan, Polygonscan, etc.)
* Stop polling
* Store `tx_hash` for record-keeping

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

* High network congestion (transaction stuck in mempool)
* Nonce gap created by external wallet (transaction blocked behind missing nonce)
* RPC node temporarily unavailable

**What to do:**

* Wait 5-10 minutes and poll status again (Scanner Bot may find the transaction)
* Check block explorer manually for `tx_hash` (if available in earlier `broadcasted` status)
* If transaction is truly lost, you can safely resubmit (nonce has been marked `consumed`)

---

## 4. Error Handling

### 4.1 JSON-RPC Error Format

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

### 4.2 Error Code Reference

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

### 4.3 Common Error Scenarios

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
      "expected": "<test_account_from_address>",
      "actual": "<to_address>"
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

### 4.4 Client-Side Retry Logic

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

* Invalid request format → Fix your code
* Authentication failure → Fix your API key
* Validation failure → Fix transaction parameters

**Idempotency Considerations:**

Lobby generates a unique `execution_id` for every submission. If you retry a failed submission, you will create a **new transaction** with a **new execution ID**.

To avoid duplicate submissions:

1. Store the `execution_id` immediately after receiving `202 Accepted`
2. On retry, poll the original `execution_id` first to check if it's still processing
3. Only resubmit if the original transaction reached a terminal `failed` state

---

## 5. Client Implementation Examples

### 5.1 Python Implementation

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


# Example Usage

if __name__ == "__main__":
    client = LobbyClient(LobbyConfig(
        base_url="http://localhost:3000",
        api_key="lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:<test_account_from_address>",
        from_address="<test_account_from_address>"
    ))
    
    # Submit ETH transfer
    exec_id = client.submit_transaction(
        to="<to_address>",
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

### 5.2 TypeScript Implementation

**Full-featured client with TypeScript types and error handling:**

```typescript
// Types

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

// Client Implementation

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

// Example Usage

const client = new LobbyClient({
  baseUrl: "http://localhost:3000",
  apiKey: "lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:<test_account_from_address>",
  fromAddress: "<test_account_from_address>"
});

// Submit ETH transfer
const executionId = await client.submitTransaction({
  to: "<to_address>",
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

### 5.3 Rust Implementation

**Full-featured async client with strong typing and error handling:**

```rust
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::time::sleep;

// Configuration

#[derive(Clone)]
pub struct LobbyConfig {
    pub base_url: String,
    pub api_key: String,
    pub from_address: String,
}

// Request/Response Types

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

// Client Implementation

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

// Example Usage

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = LobbyClient::new(LobbyConfig {
        base_url: "http://localhost:3000".to_string(),
        api_key: "lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:<test_account_from_address>".to_string(),
        from_address: "<test_account_from_address>".to_string(),
    });

    // Submit ETH transfer
    let execution_id = client
        .submit_transaction(
            "<to_address>", // to
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

## 6. Generating API Keys

### 6.1 The `generate_api_keys` binary (***only for testing***)

* Run the binary using:

```bash
cargo run --release --bin generate_api_keys
```

This Rust binary (present in [generate_api_keys](../lobby/src/bin/generate_api_keys.rs)) **generates API keys** for the set of accounts read from the JSON file `test_keys.json`. Here's the step-by-step flow:

1. Reads `test_keys.json` from the current working directory.
2. Iterates over each account entry in the JSON object.
3. For each account, it generates:
   * A **`client_id`**: a random UUID v4.
   * An **`api_token`**: the string `lobby_live_` followed by the first 9 characters of a random UUID (e.g. `lobby_live_3f2a1b8c9`).
   * An **`env_var`**: a numbered environment variable name like `LOBBY_API_KEY_1`, `LOBBY_API_KEY_2`, etc.
   * An **`api_key_value`**: a composite key in the format `<api_token>:<client_id>:<from_address>`.
4. Formats the API key elements into:

```bash
export LOBBY_API_KEY_<N>="<api_token>:<client_id>:<from_address>"
```

and writes the read accounts details into sequenced API_KEYS to the `.env` file.
> This is only acceptable since, the test_acounts are only for **testing**, in produciton code, the custody accounts and API KEYS needs to be stored in secure environments.

---

### 6.2 Input format (`test_keys.json`)

A top-level JSON object where each key is an account name, and each value must contain at least an `"address"` field:

```json
{
  "account1": {
    "pvt_key": "0xe74176d...",
    "pub_key": "0x045ae5f...",
    "address": "0xaf9ce11..."
  },
  "account2": {
    "pvt_key": "0x25bf69f...",
    "pub_key": "0x040d382...",
    "address": "0x5b35c45..."
  },
}
```

* The top-level value **must** be a JSON object (not an array or primitive).
* Each account entry **must** have an `"address"` string field — missing it causes a hard error.
* The account name keys (e.g. `"account_one"`) are read but not used in the output.

---

### 6.3 Expected output (`.env`)

```bash
export LOBBY_API_KEY_1="lobby_live_fe627a779:5310b127-0f51-4e73-ada1-2bfb0ce3d408:0xfea6645..."
```

Key things to note:

* `api_token` is only 9 characters of entropy after the `lobby_live_` prefix — this is quite short and is intended only for testing/development .

> This limit can be increased by modifying the code [generate_api_keys](../lobby/src/bin/generate_api_keys.rs).

---

## Conclusion

Lobby simplifies blockchain transaction signing and submission by abstracting away the complex mechanics of nonce management, key custody, and on-chain validation. With its low-latency actor-based architecture and automatic failure recovery, Lobby enables developers to focus on building applications rather than managing transaction infrastructure.

**Key Takeaways:**

* **Fire-and-forget submission** — Receive `execution_id` immediately, poll for status asynchronously
* **Automatic nonce sequencing** — No race conditions, no manual nonce tracking
* **Built-in retry logic** — Transient failures (RPC timeouts, DB hiccups) are handled transparently
* **Multi-chain support** — Single API for Ethereum, Polygon, Arbitrum, and testnets
* **Production-ready architecture** — Actor sharding, semaphore backpressure, database persistence

**Next Steps:**

1. Obtain an API key from your Lobby operator
2. Integrate the client library for your language (Python/TypeScript/Rust examples provided)
3. Test on Hoodi testnet before deploying to production
4. Monitor pipeline metrics (latency, error rates, nonce gaps)
5. Set up alerts for critical failures (broadcast errors, validator timeouts)

**Resources:**

* **GitHub Repository:** [github.com/romanticNomad/Lobby](https://github.com/romanticNomad/Lobby)
* **Issue Tracker:** [github.com/romanticNomad/Lobby/issues](https://github.com/romanticNomad/Lobby/issues)
* **License:** Apache 2.0

---

*Built with Rust, Tokio, Axum, PostgreSQL, and Redis.*  
*Designed for developers who need reliable, low-latency blockchain transaction infrastructure.*

> **End of API Doc**
---
