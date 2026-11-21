---
'@fuzdev/gro': minor
---

feat: the CLI takes the binary's grant flags in a run right after the command word, `gro check --allow-run-env=X --watch` and `gro task --allow-read=. src/lib/x.task.ts`, split off by `to_grant_flags` (`task_args.ts`); the Node gro runs the command without them and warns once on stderr, "grant flags have no effect on the Node gro, which confines nothing; only the gro binary enforces them", naming each. A grant flag before the command word, one in the run spelled `--allow-*` that isn't a grant flag, a listing one with no list, `--allow-temp` with a value, and `gro run` given `--allow-run-env` with neither `--allow-run` nor `--allow-run-in` are usage errors; after the run, a flag spelled `--allow-*` is still the command's own arg, as is any flag after a path given where the command goes; a refusal's hint keeps the grant flags after the command word, `to_task_args(cli_args, grant_flags)` taking them.
