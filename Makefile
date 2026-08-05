.PHONY: up down build test lint logs status naive-run naive-redeploy naive-forget naive-restart begin-redeploy finish-redeploy kill-worker restart-worker reset-demo partition-worker heal-worker

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

naive-redeploy:
	docker stop naive-agent

# Backwards-compatible alias for older talk notes.
naive-forget: naive-redeploy

naive-restart:
	docker compose run --rm --no-deps worker naive

# Deliberately stretch a normal Worker replacement across two stage beats. The
# old Worker stops first; start the compatible replacement after sending the
# release Signal so the audience can see that Temporal retained it.
begin-redeploy:
	docker compose stop worker

finish-redeploy:
	docker compose up -d --force-recreate worker

# Optional hard-crash controls; the primary talk path uses begin/finish-redeploy.
kill-worker:
	docker compose kill -s SIGKILL worker

restart-worker:
	docker compose up -d worker

# Network partition: the Worker process stays alive but is cut off from Temporal
# (and Stage). Workflows lose no progress and resume once the partition heals.
partition-worker:
	docker network disconnect rust-confessional_default rust-confessional-worker-1

heal-worker:
	docker network connect rust-confessional_default rust-confessional-worker-1

reset-demo:
	curl -fsS -X POST http://localhost:3000/api/demo/reset
