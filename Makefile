# Katana Desktop
#
#   make                 build the release binary
#   make install         install for the current user (Linux: ~/.local, macOS: ~/Applications)
#   make uninstall       remove what `make install` put in place (your data is kept)
#   make run | test | clean
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

SOURCES := Cargo.toml Cargo.lock $(wildcard src/*.rs) $(wildcard web/*)

.PHONY: all build run test clean install uninstall bundle

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
