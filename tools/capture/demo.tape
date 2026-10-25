# bdi draws the tree of work in two projects with the agent on each bead, then
# redraws as a new agent takes a bead and another finishes a bead and takes
# the next. record.sh runs it over the ground ground.sh lays.

Output docs/bdi-at-work.gif

Set Shell bash
Set FontFamily "JetBrains Mono"
Set FontSize 27
Set Width 1544
Set Height 780
Set Padding 36
Set Framerate 10
Set TypingSpeed 60ms
Set Theme { "name": "bdi", "background": "#12151a", "foreground": "#c3c8d1", "cursor": "#c3c8d1", "selection": "#2a2f3a", "black": "#1c1f26", "red": "#e05252", "green": "#4fa65b", "yellow": "#c9a227", "blue": "#4a8fe0", "magenta": "#a86fd0", "cyan": "#3fa8a0", "white": "#c3c8d1", "brightBlack": "#5c6370", "brightRed": "#f07171", "brightGreen": "#6cc46c", "brightYellow": "#e0c14a", "brightBlue": "#6fa8f0", "brightMagenta": "#c48fe0", "brightCyan": "#5fc8c0", "brightWhite": "#eef1f6" }

Hide
Type "cd $DEMO_GROUND && export HOME=$DEMO_GROUND PS1='$ ' && clear"
Enter
Sleep 500ms
Type "bdi --all-projects"
Enter
Wait+Screen@90s /agents/
Show
Sleep 2s
Down
Sleep 300ms
Down
Wait+Screen@30s /live keys · working/
Wait+Screen@30s /reindex cost +◍ search index/
Sleep 800ms
Down
Sleep 4s
Hide
Type "q"
