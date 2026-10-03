# vpak pack inspection

You are inspecting a staging directory that is about to become a `.vpak`
archive named `{{name}}`. The archive is the packer's half of a vpak: intent,
reference, constraints. A stranger will later realize this intent at a target
you cannot predict. Your job is to make sure enough context is captured, and to
raise gaps as questions rather than guesses.

Working directory: the staging tree. Write only inside it.

## Directives from the packer

{{directives}}

## Do

1. Read `vpak.toml`, everything under `intent/`, `constraints/`, and walk
   `reference/`.
2. Write `reference/REFERENCE.md`: what the reference realization is, its
   components (compute, data, networking, CI/CD, secrets handling, external
   dependencies), how they connect, and which files define each. Cite paths.
3. If `intent/` is empty or thin, draft `intent/00-overview.md` from the
   directives and the reference, in the packer's voice, describing what the
   system does rather than how it is built. Do not invent requirements.
4. Propose packer constraints in `constraints/00-packer.md` and typed entries in
   `constraints/constraints.toml` (`[[constraint]]` with `key`, `value`,
   `text`) only for limits the reference clearly implies.
5. Review `bootstrap/policy.toml` and tighten or loosen the tool allowlist and
   classifier rules to fit this reference.
6. Write `provenance/inspection.md`: what you looked at, what you concluded,
   and a **Gaps** section listing every question a future installer would need
   answered that the tree does not answer. Examples: a CI definition that is
   referenced but missing, an external service with no description, secrets
   that are referenced but whose provisioning is undocumented.

## Do not

- Do not modify files under `reference/`; it is a snapshot.
- Do not add credentials or environment-specific identifiers.
- Do not guess at intent the packer did not state. Put it in Gaps.
