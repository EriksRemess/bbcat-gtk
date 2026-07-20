PREFIX ?= /usr/local
DESTDIR ?=
CARGO ?= cargo

BINDIR = $(DESTDIR)$(PREFIX)/bin
DATADIR = $(DESTDIR)$(PREFIX)/share

.PHONY: build deb install uninstall

build:
	$(CARGO) build --release --locked

deb:
	./scripts/build-deb

install: build
	install -Dm755 target/release/bbcat-gtk $(BINDIR)/bbcat-gtk
	install -Dm644 bbcat.desktop $(DATADIR)/applications/bbcat.desktop
	install -Dm644 bbcat.xml $(DATADIR)/mime/packages/bbcat.xml
	@if [ -z "$(DESTDIR)" ] && command -v update-mime-database >/dev/null; then \
		update-mime-database "$(PREFIX)/share/mime"; \
	fi
	@if [ -z "$(DESTDIR)" ] && command -v update-desktop-database >/dev/null; then \
		update-desktop-database "$(PREFIX)/share/applications"; \
	fi

uninstall:
	rm -f $(BINDIR)/bbcat-gtk
	rm -f $(DATADIR)/applications/bbcat.desktop
	rm -f $(DATADIR)/mime/packages/bbcat.xml
	@if [ -z "$(DESTDIR)" ] && command -v update-mime-database >/dev/null; then \
		update-mime-database "$(PREFIX)/share/mime"; \
	fi
	@if [ -z "$(DESTDIR)" ] && command -v update-desktop-database >/dev/null; then \
		update-desktop-database "$(PREFIX)/share/applications"; \
	fi
