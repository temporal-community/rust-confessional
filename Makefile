.PHONY: up down build test lint logs status naive-run naive-forget naive-restart kill-worker restart-worker reset-demo

build:
	docker compose build

test:
	docker build --target test --tag rust-confessional-test .

lint:
	docker build --target lint --tag rust-confessional-lint .

up:
	docker compose up --build -d

down:
	docker compose down

logs:
	docker compose logs -f stage worker

status:
	docker compose ps

naive-run:
	docker compose run --name naive-agent --rm --no-deps worker naive "I fixed the race condition with a sleep."

naive-forget:
	docker kill --signal KILL naive-agent

naive-restart:
	docker compose run --rm --no-deps worker naive

kill-worker:
	docker compose kill -s SIGKILL worker

restart-worker:
	docker compose up -d worker

reset-demo:
	curl -fsS -X POST http://localhost:3000/api/demo/reset
