.PHONY: lobbyup db api reset

# Default: everything a new contributor needs
lobbyup: db api
	@echo "Lobby Setup complete."

# 1. Infrastructure (idempotent)
db:
	@echo "Starting PostgreSQL and Redis..."
	cp .env.example .env
	cd database && docker compose up -d
	cd database && sqlx migrate run

# 3. Generate API keys and create .env (skip if present)
api:
	@echo "Generating API keys and updating .env ..."
	cargo run --release --quiet --bin generate_api_keys

# 4. reset the lobby setup
reset: 
	cd database && docker compose down -v
