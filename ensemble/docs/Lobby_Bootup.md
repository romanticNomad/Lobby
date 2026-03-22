# Running Lobby Locally

```bash
# 1. Clone repository
git clone https://github.com/romanticNomad/Lobby.git
cd lobby

# 2. Start PostgreSQL and Redis servers on Docker.
cd database
docker compose up -d
sqlx migrate run

# 3. Configure environment
cat > .env << EOF
DATABASE_URL=postgresql://postgres:password@localhost/lobby
SERVER_ADDR=0.0.0.0:3000
REDIS_URL=redis://localhost:6379
RUST_LOG=info

# API key (example)
LOBBY_API_KEY_1=lobby_live_dev123:550e8400-e29b-41d4-a716-446655440000:<test_account_from_address>

# RPC endpoints (get keys from Alchemy/Infura)
RPC_ENDPOINT_1=https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY
RPC_ENDPOINT_560048=https://eth-hoodi.g.alchemy.com/v2/YOUR_KEY
EOF

# 4. Create test keys
cat > test_keys.json << EOF
{
  "test_account": {
    "pvt_key": "<test_account_pvt_key>",
    "pub_key": "<test_account_pub_key>",
    "address": "test_account_from_address"
  }
}
EOF

# 5. Expose environmnet variables to terminal (Bash)
source .env

# 6. Start Lobby
cargo run --release

# Expected outcome
INFO     ｉ [info]: "relation \"_sqlx_migrations\" already exists, skipping"
INFO     ｉ [info]: api_keys loaded: 1
INFO     ｉ [info]: rpc_endpoints loaded: [ChainId(560048), ChainId(1)]
INFO     ｉ [info]: custody accounts loaded: 1
INFO     ｉ [info]: StatusRegistry loaded | artifacts_count: 0
INFO     ｉ [info]: cortex online
INFO     ｉ [info]: bots spawned: monitoring status
INFO     ｉ [info]: lobby listening at: | address: 0.0.0.0:3000

# 7. Test submission
curl -X POST http://localhost:3000/v1/transactions \
  -H "Authorization: Bearer lobby_live_dev123:550e8400-e29b-41d4-a716-446655440000:test_account_from_address" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_sendTransaction",
    "params": [{
      "from": "test_account_from_address",
      "to": "<to_address>",
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
*Built with Rust, Tokio, Axum, PostgreSQL, and Redis.*  
*Designed for developers who need reliable, low-latency blockchain transaction infrastructure.*

> **End of Lobby Bootup Guide** 
---