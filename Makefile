# Katana Desktop
#
#   make                 build the release binary
#   make install         install both frontends for the current user
#   make uninstall       remove what `make install` put in place (your data is kept)
#   make install-desktop | uninstall-desktop
#                        only the desktop app (Linux: ~/.local, macOS: ~/Applications)
#   make run | test | clean
#
# katana-tui, the terminal frontend, can be built and installed on its own (it needs no webview):
#   make build-tui | run-tui
#   make install-tui | uninstall-tui
#                        only katana-tui, in $(PREFIX)/bin (~/.local/bin)
#
#   make release ...     set the version, commit and tag; pushing the tag publishes the release
#                        major|minor|patch bumps that part (zeroing the smaller ones); on an -rcN
#                          version it releases that version instead, if it is already such a bump
#                        rc starts or advances a release candidate: x.y.z-rc1, -rc2, ...
#                        x.y.z or x.y.z-rcN sets it
#                        (V=... works as well)
#   make windows         cross-build dist/KatanaDesktop-<version>-setup.exe and -portable.exe
#                        (needs mingw-w64-gcc, nsis and `rustup target add x86_64-pc-windows-gnu`)
#   make windows-tui     cross-build dist/katana-tui-<version>-windows-x86_64.zip (needs zip, not nsis)
#
# Linux system-wide:   make build build-tui && sudo make install PREFIX=/usr/local
# macOS elsewhere:     make install APPDIR=/Applications

NAME    := katana-desktop
APPNAME := Katana Desktop
BIN     := target/release/$(NAME)
TUI     := target/release/katana-tui
UNAME   := $(shell uname -s)

PREFIX  ?= $(HOME)/.local
DESTDIR ?=
BINDIR  := $(DESTDIR)$(PREFIX)/bin
DATADIR := $(DESTDIR)$(PREFIX)/share
APPDIR  ?= $(HOME)/Applications

SOURCES := Cargo.toml Cargo.lock build.rs $(wildcard src/*.rs) $(wildcard src/tui/*.rs) $(wildcard web/*)

.PHONY: all build run test clean install uninstall install-desktop uninstall-desktop bundle windows release
.PHONY: build-tui run-tui install-tui uninstall-tui windows-tui

all: build

install: install-desktop install-tui

uninstall: uninstall-desktop uninstall-tui

build: $(BIN)

$(BIN): $(SOURCES)
	cargo build --release --bin $(NAME)

run:
	cargo run --release

build-tui: $(TUI)

$(TUI): $(SOURCES)
	cargo build --release --no-default-features --features tui --bin katana-tui

run-tui:
	cargo run --release --no-default-features --features tui --bin katana-tui

install-tui: $(TUI)
	install -d "$(BINDIR)"
	install -m755 $(TUI) "$(BINDIR)/katana-tui"
	@echo "Installed $(PREFIX)/bin/katana-tui"

uninstall-tui:
	rm -f "$(BINDIR)/katana-tui"
	@echo "Uninstalled. Your data is kept."

test:
	cargo test

clean:
	cargo clean
	rm -f dist/KatanaDesktop-*.exe dist/katana-tui-*.zip  # only the build outputs: dist may be a symlink to a share

# ---- Releases: .github/workflows/release.yml builds and publishes everything for a pushed v* tag
# `make release patch`: the word after `release` is the version, not a target to build
ifeq (release,$(firstword $(MAKECMDGOALS)))
ifneq (,$(word 2,$(MAKECMDGOALS)))
V := $(or $(V),$(word 2,$(MAKECMDGOALS)))
.PHONY: $(word 2,$(MAKECMDGOALS))
$(word 2,$(MAKECMDGOALS)): release
	@:
endif
endif

release:
	@set -e; \
	semver='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-rc[1-9][0-9]*)?$$'; \
	usage="usage: make release major|minor|patch|rc|x.y.z[-rcN] (got '$(V)')"; \
	cur='$(VERSION)'; \
	case '$(V)' in major|minor|patch|rc) \
		echo "$$cur" | grep -Eq "$$semver" || { echo "cannot bump: current version '$$cur' is not x.y.z[-rcN]"; exit 1; }; \
		core=$${cur%%-*}; rc=; case "$$cur" in *-rc*) rc=$${cur##*-rc} ;; esac; \
		major=$${core%%.*}; minor=$${core#*.}; minor=$${minor%%.*}; patch=$${core##*.} ;; \
	esac; \
	case '$(V)' in \
		major) if [ -n "$$rc" ] && [ "$$minor.$$patch" = 0.0 ]; then new=$$core; else new=$$((major + 1)).0.0; fi ;; \
		minor) if [ -n "$$rc" ] && [ "$$patch" = 0 ]; then new=$$core; else new=$$major.$$((minor + 1)).0; fi ;; \
		patch) if [ -n "$$rc" ]; then new=$$core; else new=$$major.$$minor.$$((patch + 1)); fi ;; \
		rc)    if [ -n "$$rc" ]; then new=$$core-rc$$((rc + 1)); \
		       elif git rev-parse -q --verify "refs/tags/v$$cur" >/dev/null; then new=$$major.$$minor.$$((patch + 1))-rc1; \
		       else new=$$cur-rc1; fi ;; \
		*)     new='$(V)' ;; \
	esac; \
	echo "$$new" | grep -Eq "$$semver" || { echo "$$usage"; exit 1; }; \
	git diff --quiet HEAD || { echo "commit or stash your changes first"; exit 1; }; \
	echo "Releasing v$$new (was v$$cur)"; \
	sed -i.bak 's/^version = ".*"/version = "'"$$new"'"/' Cargo.toml && rm Cargo.toml.bak; \
	cargo update --workspace --offline; \
	git commit -qam "Release v$$new"; \
	git tag -a "v$$new" -m "v$$new"; \
	echo "Tagged v$$new. Publish it with: git push origin HEAD v$$new"

# ---- Windows releases, cross-compiled with MinGW and packaged with NSIS:
#   dist/KatanaDesktop-<version>-setup.exe     per-user installer (Start menu, Apps & features)
#   dist/KatanaDesktop-<version>-portable.exe  single-file, runs without installing
# MinGW builds load WebView2Loader.dll at runtime (MSVC builds link it statically),
# so both packages carry the DLL next to the app.
WIN_TARGET   := x86_64-pc-windows-gnu
WIN_EXE      := target/$(WIN_TARGET)/release/$(NAME).exe
WIN_STAGE    := target/windows-stage
VERSION      := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
WIN_SETUP    := dist/KatanaDesktop-$(VERSION)-setup.exe
WIN_PORTABLE := dist/KatanaDesktop-$(VERSION)-portable.exe
# Windows version resources must be numeric (x.y.z.0), so an -rcN suffix is dropped there
NSIS_DEFS     = -V2 -DVERSION=$(VERSION) -DFILEVERSION=$(firstword $(subst -, ,$(VERSION))).0 -DSRCDIR=$(abspath $(WIN_STAGE)) -DICON=$(abspath web/icon.ico) -DWIZARD=$(abspath installer/wizard.bmp)

windows: $(WIN_SETUP) $(WIN_PORTABLE)

$(WIN_EXE): $(SOURCES)
	cargo build --release --target $(WIN_TARGET) --bin $(NAME)

$(WIN_STAGE)/$(NAME).exe: $(WIN_EXE)
	rm -rf $(WIN_STAGE) && mkdir -p $(WIN_STAGE)
	cp $(WIN_EXE) $(WIN_STAGE)/
	cp "$$(ls target/$(WIN_TARGET)/release/build/webview2-com-sys-*/out/x64/WebView2Loader.dll | head -n1)" $(WIN_STAGE)/

$(WIN_SETUP): $(WIN_STAGE)/$(NAME).exe installer/setup.nsi installer/wizard.bmp web/icon.ico
	mkdir -p dist
	makensis $(NSIS_DEFS) -DOUTFILE=$(abspath $@) installer/setup.nsi
	@echo "Built $@"

$(WIN_PORTABLE): $(WIN_STAGE)/$(NAME).exe installer/portable.nsi web/icon.ico
	mkdir -p dist
	makensis $(NSIS_DEFS) -DOUTFILE=$(abspath $@) installer/portable.nsi
	@echo "Built $@"

# katana-tui for Windows is a single console program: no installer, just the .exe in a .zip
WIN_TUI_EXE := target/$(WIN_TARGET)/release/katana-tui.exe
WIN_TUI     := dist/katana-tui-$(VERSION)-windows-x86_64.zip

windows-tui: $(WIN_TUI)

$(WIN_TUI_EXE): $(SOURCES)
	cargo build --release --target $(WIN_TARGET) --no-default-features --features tui --bin katana-tui

$(WIN_TUI): $(WIN_TUI_EXE) LICENSE
	mkdir -p dist
	rm -f $@
	zip -j $@ $(WIN_TUI_EXE) LICENSE
	@echo "Built $@"

ifeq ($(UNAME),Darwin)

bundle: $(BIN)
	./scripts/bundle-macos.sh

install-desktop: bundle
	mkdir -p "$(APPDIR)"
	rm -rf "$(APPDIR)/$(APPNAME).app"
	cp -R "target/$(APPNAME).app" "$(APPDIR)/"
	@echo "Installed $(APPDIR)/$(APPNAME).app"

uninstall-desktop:
	rm -rf "$(APPDIR)/$(APPNAME).app"
	@echo "Removed $(APPDIR)/$(APPNAME).app"
	@echo "Your data is kept in ~/Library/Application Support/$(NAME)"

else

install-desktop: $(BIN)
	install -Dm755 $(BIN) "$(BINDIR)/$(NAME)"
	install -Dm644 web/icon.png "$(DATADIR)/icons/hicolor/256x256/apps/$(NAME).png"
	install -d "$(DATADIR)/applications"
	printf '%s\n' \
		'[Desktop Entry]' \
		'Type=Application' \
		'Name=$(APPNAME)' \
		'GenericName=Nonograms' \
		'Comment=Unofficial client for Nonograms Katana user puzzles' \
		'Keywords=nonogram;nonograms;katana;griddlers;picross;hanjie;puzzle;' \
		'Exec=$(PREFIX)/bin/$(NAME)' \
		'Icon=$(NAME)' \
		'Categories=Game;LogicGame;' \
		'StartupWMClass=$(NAME)' \
		> "$(DATADIR)/applications/$(NAME).desktop"
	@echo "Installed $(PREFIX)/bin/$(NAME) and a \"$(APPNAME)\" menu entry"

uninstall-desktop:
	rm -f "$(BINDIR)/$(NAME)"
	rm -f "$(DATADIR)/icons/hicolor/256x256/apps/$(NAME).png"
	rm -f "$(DATADIR)/applications/$(NAME).desktop"
	@echo "Uninstalled. Your data is kept in ~/.local/share/$(NAME)"

endif
