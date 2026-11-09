# Reproducible README demo. Run from the repository root via assets/demo/record.sh.
Output assets/demo/demo.gif

Require sati
Set Shell "bash"
Set Width 1200
Set Height 675
Set FontSize 26
Set FontFamily "Lilex Nerd Font Mono"
Set Framerate 20
Set TypingSpeed 70ms

# Reset only the demo checklist. Leave tasks.lock in place: sati uses it for locking.
# record.sh sets the working directory to the repository root.
Hide
Type "rm -f assets/demo/data/tasks.json"
Enter
Sleep 300ms
Type "sati --config assets/demo/config.toml --data-dir assets/demo/data"
Enter
Sleep 1s

# Populate the list using sati itself, without displaying the setup.
Type "a"
Type "plan the week"
Enter
Sleep 200ms
Type "a"
Type "work: review pull request"
Enter
Sleep 200ms
Type "a"
Type "work: prepare release notes"
Enter
Sleep 200ms
Type "a"
Type "work: fix login bug"
Enter
Sleep 200ms
Type "a"
Type "work: update release checklist"
Enter
Sleep 200ms
Type "a"
Type "work: triage open issues"
Enter
Sleep 200ms
Type "a"
Type "work: review ci pipeline"
Enter
Sleep 200ms
Type "a"
Type "personal: read a chapter"
Enter
Sleep 200ms
Type "a"
Type "personal: go for a walk"
Enter
Sleep 300ms
Type "g"
Sleep 300ms

# 1. Establish the interface and its section tabs.
Show
Sleep 2.5s

# 2. Add and complete a task.
Type "a"
Sleep 300ms
Type "buy groceries"
Enter
Sleep 1.5s
Space
Sleep 1.3s

# 3. Reorder the selected task and move around the list.
Type "K"
Sleep 900ms
Type "k"
Sleep 300ms
Type "j"
Sleep 650ms

# 4. Visit unsectioned, work, and personal tabs, then return to work.
Tab
Sleep 700ms
Tab
Sleep 1.2s
Tab
Sleep 1s
Type "h"
Sleep 700ms

# 5. Search within work: Tab cycles between the two tasks matching "release".
Type "/"
Sleep 200ms
Type "release"
Sleep 900ms
Tab
Sleep 900ms
Tab
Sleep 900ms
Enter
Sleep 600ms

# 6. Show the built-in keyboard reference, then return to the checklist.
Type "?"
Sleep 2.7s
Escape
Sleep 400ms
Type "h"
Sleep 300ms
Type "h"
Type "g"
Sleep 2s
