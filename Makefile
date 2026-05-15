.PHONY: lobbyup db api reset

# Default: everything a new contributor needs
lobbyup: db api
	@echo "Lobby Setup complete."

# 1. Infrastructure (idempotent)
db:
	@echo "Setting up Environment"
	cp .env.example .env
	cd database && docker compose up -d
	cd database && sqlx migrate run

# 3. Generate API keys and create .env (skip if present)
api:
	@echo "Generating API keys for the test-accounts"
	@start=$$(date +%s); TIMER_PID=""; \
	if [ -t 2 ]; then \
		while true; do \
			elapsed=$$(($$(date +%s) - start)); \
			printf "\r⏳ Compiling and generating keys:    (%ds)\n" $$elapsed >&2; \
			sleep 1; \
		done & TIMER_PID=$$!; \
		trap 'kill $$TIMER_PID 2>/dev/null; printf "\n" >&2' INT TERM; \
	fi; \
	cargo run --release --quiet --bin generate_api_keys; \
	EXIT_CODE=$$?; \
	if [ -n "$$TIMER_PID" ]; then \
		kill $$TIMER_PID 2>/dev/null; \
		wait $$TIMER_PID 2>/dev/null; \
	fi; \
	exit $$EXIT_CODE

# 4. reset the lobby setup
reset: 
	cd database && docker compose down -v
