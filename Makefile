# Katana Desktop
#
#   make                 build the release binary
#   make install         install for the current user (Linux: ~/.local, macOS: ~/Applications)
#   make uninstall       remove what `make install` put in place (your data is kept)
#   make run | test | clean
#   make windows         cross-build dist/KatanaDesktop-<version>-setup.exe and -portable.exe
#                        (needs mingw-w64-gcc, nsis and `rustup target add x86_64-pc-windows-gnu`)
#
# Linux system-wide:   make && sudo make install PREFIX=/usr/local
# macOS elsewhere:     make install APPDIR=/Applications

NAME    := katana-desktop
APPNAME := Katana Desktop
BIN     := target/release/$(NAME)
UNAME   := $(shell uname -s)

PREFIX  ?= $(HOME)/.local
DESTDIR ?=
BINDIR  := $(DESTDIR)$(PREFIX)/bin
DATADIR := $(DESTDIR)$(PREFIX)/share
APPDIR  ?= $(HOME)/Applications

SOURCES := Cargo.toml Cargo.lock build.rs $(wildcard src/*.rs) $(wildcard web/*)

.PHONY: all build run test clean install uninstall bundle windows

all: build

build: $(BIN)

$(BIN): $(SOURCES)
	cargo build --release

run:
	cargo run --release

test:
	cargo test

clean:
	cargo clean
	rm -rf dist

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
NSIS_DEFS     = -V2 -DVERSION=$(VERSION) -DSRCDIR=$(abspath $(WIN_STAGE)) -DICON=$(abspath web/icon.ico) -DWIZARD=$(abspath installer/wizard.bmp)

windows: $(WIN_SETUP) $(WIN_PORTABLE)

$(WIN_EXE): $(SOURCES)
	cargo build --release --target $(WIN_TARGET)

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

ifeq ($(UNAME),Darwin)

bundle: $(BIN)
	./scripts/bundle-macos.sh

install: bundle
	mkdir -p "$(APPDIR)"
	rm -rf "$(APPDIR)/$(APPNAME).app"
	cp -R "target/$(APPNAME).app" "$(APPDIR)/"
	@echo "Installed $(APPDIR)/$(APPNAME).app"

uninstall:
	rm -rf "$(APPDIR)/$(APPNAME).app"
	@echo "Removed $(APPDIR)/$(APPNAME).app"
	@echo "Your data is kept in ~/Library/Application Support/$(NAME)"

else

install: $(BIN)
	install -Dm755 $(BIN) "$(BINDIR)/$(NAME)"
	install -Dm644 web/icon.png "$(DATADIR)/icons/hicolor/256x256/apps/$(NAME).png"
	install -d "$(DATADIR)/applications"
	printf '%s\n' \
		'[Desktop Entry]' \
		'Type=Application' \
		'Name=$(APPNAME)' \
		'GenericName=Nonograms' \
		'Comment=Nonograms Katana user puzzles on the desktop' \
		'Keywords=nonogram;nonograms;katana;griddlers;picross;hanjie;puzzle;' \
		'Exec=$(PREFIX)/bin/$(NAME)' \
		'Icon=$(NAME)' \
		'Categories=Game;LogicGame;' \
		'StartupWMClass=$(NAME)' \
		> "$(DATADIR)/applications/$(NAME).desktop"
	@echo "Installed $(PREFIX)/bin/$(NAME) and a \"$(APPNAME)\" menu entry"

uninstall:
	rm -f "$(BINDIR)/$(NAME)"
	rm -f "$(DATADIR)/icons/hicolor/256x256/apps/$(NAME).png"
	rm -f "$(DATADIR)/applications/$(NAME).desktop"
	@echo "Uninstalled. Your data is kept in ~/.local/share/$(NAME)"

endif
