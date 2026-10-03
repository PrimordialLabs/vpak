# Phase: verify

Install of `{{name}}`. Working directory: `{{workdir}}`. Destination: `{{dest}}`.

Answer: does the realization satisfy the intent?

1. Re-read the intent in the bootstrap and `vpak constraint list`.
2. Check each intent statement against the realization with read-only
   commands: health endpoints, resource listings, CI definitions present,
   secrets obtained the way the target requires.
3. Write the verification report:
   `vpak journal append --phase verify --title "verification" --body "..."`
   listing each intent statement with pass, fail, or not verifiable, and each
   constraint with honored or violated.
4. If something fails, loop back: `vpak phase set plan --reason "verify found
   ..."` and add the corrective steps there.
5. When everything passes, record the realized target for a future re-pack
   with `vpak target show` reviewed and complete, then
   `vpak phase set done --reason "verified"`.
