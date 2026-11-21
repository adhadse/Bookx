PREFIX ?= /usr/local
JOBS ?= $(shell nproc)
APP_ID = com.adhadse.Bookx
TARGET_DIR ?= target/release
LANGS = $(basename $(notdir $(wildcard po/*.po)))
# Cargo features for `make build`, for example CARGO_FEATURES=bookx/reviewed-locales-only (the Flathub build:
# embeds and installs only the languages in po/reviewed-locales.txt, docs/translations-hosting.md).
CARGO_FEATURES ?=
ifneq (,$(findstring reviewed-locales-only,$(CARGO_FEATURES)))
LANGS = $(shell sed 's/[#].*//' po/reviewed-locales.txt | tr -s ' \t\n' ' ')
endif
FEATURE_FLAG = $(if $(CARGO_FEATURES),--features $(CARGO_FEATURES))

.PHONY: build run test perf scale ui-test mockups install install-flatpak uninstall flatpak flatpak-sources desktop-check i18n locale release-check tarball

build:
	cargo build --release $(FEATURE_FLAG)

run:
	cargo run --release

test:
	cargo test

perf:
	cargo test --release --workspace --test perf_budgets
	cargo test --release -p bookx --bin bookx perf::

# the full table of timings at 10k, 50k and 100k books (docs/performance.md)
scale:
	cargo run --release -p bookx-store --example scale -- 10000 50000 100000

ui-test:
	JOBS=$(JOBS) build-aux/ui-tests.sh

mockups:
	cargo run --example mockups

install: build
	cargo build --release -p bookx-cli
	$(MAKE) install-flatpak

# Installs what is already built in $(TARGET_DIR) (also used by the Flatpak manifest).
install-flatpak:
	install -Dm755 $(TARGET_DIR)/bookx $(DESTDIR)$(PREFIX)/bin/bookx
	if [ -f $(TARGET_DIR)/bookx-cli ]; then install -Dm755 $(TARGET_DIR)/bookx-cli $(DESTDIR)$(PREFIX)/bin/bookx-cli; fi
	install -Dm644 data/$(APP_ID).desktop $(DESTDIR)$(PREFIX)/share/applications/$(APP_ID).desktop
	install -Dm644 data/$(APP_ID).metainfo.xml $(DESTDIR)$(PREFIX)/share/metainfo/$(APP_ID).metainfo.xml
	install -Dm644 data/$(APP_ID).mime.xml $(DESTDIR)$(PREFIX)/share/mime/packages/$(APP_ID).xml
	install -Dm644 data/icons/hicolor/scalable/apps/$(APP_ID).svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/$(APP_ID).svg
	install -Dm644 data/icons/hicolor/scalable/apps/$(APP_ID)-symbolic.svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/symbolic/apps/$(APP_ID)-symbolic.svg
	$(MAKE) locale DESTDIR=$(DESTDIR) PREFIX=$(PREFIX)

# Compiled translations (the same strings are embedded in the program; these add Weblate languages).
locale:
	@if command -v msgfmt >/dev/null; then for l in $(LANGS); do \
	  install -d $(DESTDIR)$(PREFIX)/share/locale/$$l/LC_MESSAGES && \
	  msgfmt -o $(DESTDIR)$(PREFIX)/share/locale/$$l/LC_MESSAGES/bookx.mo po/$$l.po; done; fi

i18n:
	build-aux/i18n.sh

desktop-check:
	desktop-file-validate data/$(APP_ID).desktop
	appstreamcli validate --no-net data/$(APP_ID).metainfo.xml
	xmllint --noout data/$(APP_ID).mime.xml

release-check:
	build-aux/release-check.sh

tarball: build
	cargo build --release -p bookx-cli
	build-aux/tarball.sh

flatpak-sources:
	build-aux/flatpak-cargo-sources.py

flatpak:
	build-aux/flatpak.sh

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/bookx \
	      $(DESTDIR)$(PREFIX)/share/applications/$(APP_ID).desktop \
	      $(DESTDIR)$(PREFIX)/share/metainfo/$(APP_ID).metainfo.xml \
	      $(DESTDIR)$(PREFIX)/share/mime/packages/$(APP_ID).xml \
	      $(DESTDIR)$(PREFIX)/bin/bookx-cli \
	      $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/$(APP_ID).svg \
	      $(DESTDIR)$(PREFIX)/share/icons/hicolor/symbolic/apps/$(APP_ID)-symbolic.svg
