# BeautifulRust skills

Source: https://github.com/morganlinton/BeautifulRust

Pinned revision: `a2b22682fa69b5e2d41a5d041663df7d51aac521`.

This project installs `rust-best-practices`, `rust-review`, and `rust-port`.
The source skill text and all five best-practice references are unchanged.
The upstream MIT license is in `BeautifulRust.LICENSE`.
The source manifest records each installed file's Git blob hash.
Evaluation fixtures and repository automation are not needed to run these skills.

Use the review and best-practice skills for Rust changes. Use the port skill only for
language migration. Follow the project's product contracts and crate boundaries.
Keep review recommendations proportionate to this application. In particular, do not
add crates, change wire types, or remove deliberate ownership guarantees only for style.
The workspace already defines lints. Review proposed lint additions before enabling them.
