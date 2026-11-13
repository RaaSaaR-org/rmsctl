Output tapes/upload.gif

Set Shell zsh
Set FontSize 14
Set Width 1100
Set Height 700
Set Theme "Catppuccin Mocha"
Set Padding 20
Set TypingSpeed 50ms

# Without --yes: what would be sent, and nothing else
Type "rmsctl datasets upload ./g1-dex3-bottle-sorting-sim"
Enter
Sleep 4s

Type "clear"
Enter
Sleep 0.5s

# The upload itself: parts, progress, the server's verdict
Type "rmsctl datasets upload ./g1-dex3-bottle-sorting-sim --name 'G1 Dex3 bottle sorting' --yes"
Enter
Sleep 30s

Type "clear"
Enter
Sleep 0.5s

Type "rmsctl datasets list"
Enter
Sleep 3s
