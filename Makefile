.PHONY: lobbyup db keys api reset

KEYS_FILE := test_keys.json
ENV_FILE := .env

# Default: everything a new contributor needs
lobbyup: db keys api
	@echo "Lobby Setup complete."

# 1. Infrastructure (idempotent)
db:
	@echo "Starting PostgreSQL and Redis..."
	cp .env.example .env
	cd database && docker compose up -d
	cd database && sqlx migrate run

# 2. Test keys (copy example if missing, skip if present)
keys:
	@if [ ! -f $(KEYS_FILE) ]; then \
		echo "Copying example test keys..."; \
		cp test_keys.json.example $(KEYS_FILE); \
	else \
		echo "Test keys already exist."; \
	fi

# 3. Generate API keys and create .env (skip if present)
api: keys
	@echo "Generating API keys and updating .env..."
	cargo run --release --quiet --bin generate_api_keys

# 4. reset the lobby setup
reset: 
	cd database && docker compose down -v
	rm -f .env && rm -f test_keys.json
