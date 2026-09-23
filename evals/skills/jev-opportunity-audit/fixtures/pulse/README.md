# Pulse

Pulse is a shared inbox for support teams. It pulls mail out of a mailbox,
threads it into conversations, puts each conversation in front of the right
group of people, and gives those people the context they need to answer.

## What it does

- **Ingest.** IMAP and inbound SMTP webhooks land in `pulse.ingest`. Messages
  are threaded by `Message-ID` / `In-Reply-To` so a reply never opens a second
  ticket.
- **Intake.** New conversations get a queue and an urgency score before anyone
  sees them, so the morning shift starts at the top of a sorted list rather
  than at the top of a pile.
- **Knowledge base.** `pulse.kb` searches published help articles and internal
  runbooks and attaches the useful ones to the conversation.
- **Composing.** `pulse.compose` offers the agent a draft. Nothing is ever sent
  without a human pressing send.
- **Assistant.** `pulse.agent` is the opt-in assistant that can look things up
  in the billing and shipping systems on the agent's behalf.

## Running it locally

```
python -m venv .venv && . .venv/bin/activate
pip install -r requirements.txt
cp .env.template .env        # you will need OPENAI_API_KEY and a Postgres URL
alembic upgrade head
uvicorn pulse.api:app --reload
```

The web client lives in `web/`. `npm install && npm run dev` proxies to the
API on port 8000.

## Layout

```
pulse/agent      assistant loop and the tools it may call
pulse/auth       roles, scopes, and the checks that use them
pulse/billing    invoices, refunds, money formatting
pulse/compose    reply drafting
pulse/config     plan tiers, regions, feature flags
pulse/ingest     mail parsing and threading
pulse/intake     queue assignment and urgency
pulse/kb         article search
```

## Conventions

Tests live beside the code as `test_*.py`. We run `ruff` and `mypy --strict`
on `pulse/` in CI; `web/` is checked with `tsc --noEmit` and `eslint`.
