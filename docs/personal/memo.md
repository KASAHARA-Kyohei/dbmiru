# Personal memo — DbMiru workflow (easy-to-digest version)

## Project assumptions

- **Codex writes the code**
- I focus on **decisions, validation, and direction**
- Specs (md files) **take priority over code**
- Only I check items off with `[x]`

---

## Milestone overview (just remember this)

M0: Ship something that launches (keepable Hello World)  
M1: Execute queries (SELECT 1 succeeds)  
M2: View data (tables + contents visible)  
M3: Grow the project (workspace + DB abstraction)  
M4: Delightful to use (history, copy, smooth UX)

### What each milestone feels like

- **M0** → “This project can continue.”
- **M1** → “It works as a DB client.”
- **M2** → “Usable for real work.”
- **M3** → “Future expansion won’t break it.”
- **M4** → “It’s fun to touch.”

Knowing that later milestones exist is enough for now.

---

## Daily routine (critical)

### Start of session (~5 min)

1. Open `docs/status.md`
2. Confirm the current milestone
3. Pick exactly **one** checklist item to tackle next
4. Ask Codex to handle that single item

### End of session (~5 min)

1. Use the app yourself
2. Decide if it “feels OK” to use
3. Mark `[x]` in `docs/milestones.md` only for items that truly pass
4. Lightly update `status.md`
5. `git commit`

---

## Checklist rules

- `[x]` = **my decision that we can move forward**
- If Codex checks something off, I can revert it
- “Mostly works” stays `[ ]`
- Any doubt → keep `[ ]`

---

## Division of roles with Codex

### Codex

- Implement features
- Report “I believe this checklist item is satisfied”
- Write manual verification steps

### Me

- Actually verify behavior
- Decide whether to check the box
- Stop things if direction drifts

---

## Core idea of spec-first development

- The goal isn’t to write md files
- The goal is to **define “what done looks like” first**
- Code follows the spec

When in doubt, ask:

> “Does this matter for the current milestone?”

---

## When stuck

Don’t do:

- Add features
- Make large architectural changes
- Chase perfection

Do this instead:

1. Read `AGENTS.md`
2. Read the DoD section in `docs/milestones.md`
3. Choose one “smallest forward step”

---

## Personal reminders

- Defer workspace splitting until **M2/M3**
- Don’t worry about future DB engines yet
- “Seems convenient” is usually a trap
- If it stops being fun, pause

---

## How to use this memo

- Read in the morning
- Revisit when unsure
- Re-read after weeks away

→ **Makes it easy to resume**

---

## Final note to self

- This isn’t work
- No need to build the perfect answer
- “Being able to continue” is success

**Whatever I finished today is enough.**

---

## Other handy personal docs (add when needed)

- `decisions.md` → one-liner rationale for choices
- `ideas-later.md` → parking lot for “not now” ideas
- `gotchas.md` → trap notes (env issues, etc.)
