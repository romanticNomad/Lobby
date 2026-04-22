.PHONY: all setup db keys api run clean

KEYS_FILE := test_keys.json
ENV_FILE := .env

# Default: everything a new contributor needs
all: db keys api
	@echo "Setup complete. Run 'make run' to start Lobby."

# 1. Infrastructure (idempotent)
db:
	@echo "Starting PostgreSQL and Redis..."
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
	@if [ ! -f $(ENV_FILE) ]; then \
		echo "Generating API keys and creating .env..."; \
		cargo run --release --quiet --bin generate_api_keys; \
	else \
		echo ".env already exists."; \
	fi

# 4. Run the application
run: db
	source $(ENV_FILE) && cargo run --release --bin lobby

# Cleanup
clean:
	cd database && docker compose down -v
	rm -f $(KEYS_FILE) $(ENV_FILE)