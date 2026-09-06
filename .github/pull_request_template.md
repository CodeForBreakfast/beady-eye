The title of this pull request becomes the commit subject on `main`. The merge is
a squash, so that title is the only line of your branch `main` keeps, and a check
reads it. Get the verdict now rather than after a failed check:

    nix develop -c conventional-subject '<your title>'

It prints the type and scope lists if it refuses. CONTRIBUTING.md has the rest.
