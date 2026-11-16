# Donna: no duplicate tool text in the `codemode` section

Run with one command (Node 22.19+; it runs `npm ci` in `apps/agent` when needed):

```bash
./evals/donna-prompt/run.sh after     # the 35 cases in today's (direct) mode → evals/donna-prompt/results-after.json
```

The cases, the fake Orbit and the scripted model are copied from the code-mode eval (branch `donna-codemode-eval`,
`evals/donna-codemode`), direct mode only. The scripted model's notes on `find_tools` apply only to that eval's
collapsed mode; here every tool is offered directly. Tokens are characters / 4. `Agent`'s fourth constructor
argument (`models`) is only for this eval.

## The change

Pi's `codemode` prompt section (`createCodemodeExtension`) declares every tool a script can call, as TypeScript, up to
a 3,000-token budget. In a normal run the model is offered all those tools directly, with their full definitions, so
the section repeated ~2.9k tokens. `apps/agent/src/codemode.ts` now renders Pi's section only for the callable tools
the request does not offer, after one line: scripts call each offered tool with the same arguments, except the tools
listed as model-only (`ask_teammate`, `load_skill`, `read_skill_file`, `codemode`). In a routine's script run (which
offers only `run_saved_script`) the section is as before. Which tools a script can call, the hooks, the custom agent's
tool list, the tool-call limit and the approvals do not change: only prompt text changes.

## Results (2026-11-16)

| | Before (c62e383c) | After |
| --- | --- | --- |
| System prompt + tool definitions per request | 29,614 tokens | 26,941 (−2,673, −9.0%) |
| `codemode` section | 2,910 tokens | 237 (one line + `repair_routine_script`) |
| Input tokens per request | 29,839 | 27,166 |
| Eval success | 35/35 | 35/35 |
| Total input tokens (35 cases) | 2,745,223 | 2,499,307 (−9.0%) |
| Total tokens | 2,746,566 | 2,500,650 |
| Model requests | 92 | 92 |

Every case has the same turns and the same answers before and after.
