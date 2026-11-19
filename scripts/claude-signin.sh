#!/usr/bin/env bash
# Signs Claude Code in, in WSL, for about a year, for the Workbench and
# for your terminals.
#
# Claude Code's usual sign-in keeps itself alive by trading in a token every
# few hours, and it can lose that trade. `claude setup-token` gives a token
# that lasts about a year and is not traded in. This script asks Claude Code
# for one, keeps it in a file only you can read, and has your login shell
# load it. wbd starts in a login shell, so the agents it starts get it too.
#
# Run it in a terminal, in WSL:  bash scripts/claude-signin.sh
set -eu

kept="$HOME/.config/workbench/env"

echo "Claude Code will open a page to sign in, then print a token."
echo
claude setup-token
echo
printf 'Paste that token here and press Enter. It is not shown: '
read -rs token
echo

# A token is letters, digits, dashes and underscores. Anything else was a slip of the paste.
case "$token" in
  '' | *[!A-Za-z0-9_-]*)
    echo "That was not a token. Nothing was changed."
    exit 1
    ;;
esac

mkdir -p "$(dirname "$kept")"
(umask 077 && printf "export CLAUDE_CODE_OAUTH_TOKEN='%s'\n" "$token" > "$kept")

if ! grep -q 'workbench/env' "$HOME/.profile" 2>/dev/null; then
  printf '\n# Sign-ins the Workbench and its agents use.\n[ -f "$HOME/.config/workbench/env" ] && . "$HOME/.config/workbench/env"\n' >> "$HOME/.profile"
fi

# shellcheck disable=SC1090
. "$kept"
if claude -p "Reply with only: ok" < /dev/null > /dev/null 2>&1; then
  echo "Claude Code is signed in. The token is in $kept and lasts about a year."
  echo "Start the Workbench again so it picks it up."
else
  echo "The token was saved, but Claude Code still did not answer with it."
  echo "Run: claude -p hi   to see what it says."
  exit 1
fi
