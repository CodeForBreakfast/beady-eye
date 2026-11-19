bdi 0.40.1

Patch release, **0.40.0 → 0.40.1**.

## Highlights

**`bdi gates` waits for every GitHub Actions run on a pull request's head before it says the checks passed.** A green status from another service could make it say so while a run was still queued. There is nothing to do.

## Maintenance

bd 1.3.4 in the flake and the container image, and dependency updates.
