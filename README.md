# blazewvr

A fast, interactive command-line playground for DataWeave scripts.

## Status

Early scaffold — `run`, `repl`, and `validate` subcommands exist but are not
yet wired up. Not ready for use.

## What it is

`blazewvr` is a local CLI that runs DataWeave scripts against sample inputs
and prints colored, formatted output — a terminal-based alternative to the
browser-based DataWeave Playground, aimed at speed and keyboard-driven
workflows.

## Requirements

`blazewvr` does **not** implement DataWeave itself. It shells out to
MuleSoft's own `dw` CLI, which you must install and license separately from
MuleSoft/Salesforce. `blazewvr` looks for `dw` on your `$PATH` and does not
bundle, vendor, or redistribute it in any form.

## Disclaimer

`blazewvr` is an independent, unofficial tool built by the community. It is
**not affiliated with, endorsed by, or sponsored by Salesforce, Inc. or
MuleSoft**. DataWeave® and MuleSoft® are trademarks of Salesforce, Inc. All
product names, logos, and brands are property of their respective owners.

## License

MIT — see [LICENSE](LICENSE). The MIT license covers this project's own
source code only; it does not grant any rights to MuleSoft's `dw` CLI or the
DataWeave language, which remain governed by their own respective licenses.
