# Command line

The command `bitlog` works on a vault from the terminal. Build and install
it with Cargo:

```sh
cargo install --path crates/bitlog-cli
```

`bitlog init` creates a new vault in the current folder, or in the folder
given with `--vault`. The folder may only hold hidden files such as `.git`.

All other commands work on an existing vault. It is the folder given with
`--vault`, or else the one in the environment variable `BITLOG_VAULT`, or
else the current folder or the closest folder above it that holds a
`bitlog.toml`.

```sh
bitlog today              # today's blocks with their ids, working time and location
bitlog day 2026-09-23     # the same for another day
bitlog standup            # the last day with work and today, with your commits, to paste into a chat

bitlog block add 09:00-10:30 webshop "Checkout flow"
bitlog block edit k7f3 --time 09:00-11:00 --title "Checkout and cart"
bitlog block note k7f3    # edit the block's text in $VISUAL or $EDITOR
bitlog block rm k7f3 --move-text
bitlog set --location office
bitlog set --kind vacation --date 2026-09-25

bitlog project list
bitlog project add client-portal --name "Client portal" --color ff7800
bitlog project edit client-portal --status archived
bitlog project edit client-portal --repo ~/code/client-portal   # only on this device
bitlog project rename client-portal portal   # also in all days and notes
bitlog log client-portal --limit 5   # latest commits of that repository

bitlog task list --all    # open tasks with their ids, then the finished ones
bitlog task add "Renew the TLS certificate" --due 2026-09-30
bitlog task done h4c8     # also: drop, reopen
bitlog task edit h4c8 --title "Renew the certificates" --no-due
bitlog task move h4c8 1   # to the top of the open tasks
bitlog task archive       # move finished tasks to tasks-archive-YYYY.toml

bitlog search release deploy     # blocks, day notes, notes and tasks holding both words
bitlog search '"release notes"'  # the words as written, one after the other
bitlog stats              # time per project this month, and remote work days
bitlog stats --week       # also --year, or --from 2026-09-01 --to 2026-09-30

bitlog export blocks --from 2026-09-01 --to 2026-09-30   # CSV in exports/, all without dates
bitlog export week --date 2026-09-23                     # Markdown report of that week
bitlog export remote                                     # remote work days per year as CSV

bitlog doctor             # sync conflict copies, unknown projects, overlaps, headings in
                          # texts, wiki links without their project in block texts,
                          # broken wiki links and Git conflict markers in notes,
                          # links to missing images or out of the vault,
                          # unused and duplicate images
bitlog doctor --fix       # merge conflict copies without contradictions,
                          # escape those headings so they read as text,
                          # name the block's project in those wiki links
```

`block` and `set` change today unless `--date 2026-09-23` names another day.

`search` and `stats` use an index of the vault in `.bitlog/index.sqlite`,
which they create and bring up to date by themselves. It is never synced and
can be deleted at any time.

`doctor` finds the same problems as Check Vault in the app's main menu.

`bitlog --help` lists all commands.
