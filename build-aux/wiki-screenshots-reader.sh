#!/usr/bin/env bash
# Only the pictures of the wiki page Reading (the reader page with highlights and its popover, the sidebar, the notes
# page, the selection handles, the Wikipedia picture, the footer settings), in both styles. Same as
# build-aux/wiki-screenshots.sh with two scenarios; see there for options.
#
#   build-aux/wiki-screenshots-reader.sh
#   DRY=1 build-aux/wiki-screenshots-reader.sh
cd "$(dirname "$(readlink -f "$0")")/.." || exit 1
SCENARIOS='_wiki_reading _wiki_reader_ux' exec build-aux/wiki-screenshots.sh "$@"
