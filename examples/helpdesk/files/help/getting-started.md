# Getting Started

Welcome to the Helpdesk demo — every screen in this app was generated
from `screens.toml` + `menus.toml` by `cargo nirdosha generate-screens`.

## Roles
- **Agent** — opens, works, and moves tickets; posts to the team feed.
- **Admin** — everything an Agent can do, plus the Ops dashboard and App Settings.

## The guard is the single data authority
Every read, write, and aggregate on tickets, feed posts, and settings
routes through `GuardedTable`. What you can see (row visibility, field
masking, required/forbidden write fields, caps, tenant scoping) is
decided by the synthesized `guard_policy!` corpus — not by the screens.
If a screen offers something your role has no policy for, the guard
denies at the data plane.

## Daily flow
1. Log in (demo users: `sam`/`agent123`, `ada`/`admin123`).
2. Create a ticket on the Tickets list, or step through the wizard.
3. Move it across the Ticket Board (`open` → `in_progress` → `done`).
4. Watch the Ops Dashboard and run the Ticket Report for group counts.
5. Announce it on the Team Feed.
