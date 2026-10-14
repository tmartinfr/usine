# Agent instructions

## README

`README.md` documents everything a user can do with `usine`. Keep it in sync in the same change as the code:

- When adding, changing or removing a command, option, config key, default value or output format, update `README.md` accordingly.
- When adding a connector, add it to the connectors table, document its config section and its commands (options, examples, table and JSON output).
- Keep it user-facing: no build/test instructions, source layout, API endpoints or internal names.
- Use made-up example data (issue keys, project paths, names), never real ones.
- Before declaring a feature done, check the README against `usine --help` and the help of each subcommand.
