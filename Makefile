PREFIX ?= /usr/local
DESTDIR ?=
CARGO ?= cargo

BINDIR = $(DESTDIR)$(PREFIX)/bin
DATADIR = $(DESTDIR)$(PREFIX)/share
APP_ID = dev.bbcat.GtkViewer

.PHONY: build deb install uninstall

build:
	$(CARGO) build --release --locked

deb:
	./scripts/build-deb

install: build
	install -Dm755 target/release/bbcat-gtk $(BINDIR)/bbcat-gtk
	rm -f $(DATADIR)/applications/bbcat.desktop
	install -Dm644 $(APP_ID).desktop $(DATADIR)/applications/$(APP_ID).desktop
	install -Dm644 $(APP_ID).svg $(DATADIR)/icons/hicolor/scalable/apps/$(APP_ID).svg
	install -Dm644 bbcat.xml $(DATADIR)/mime/packages/bbcat.xml
	@if [ -z "$(DESTDIR)" ] && command -v update-mime-database >/dev/null; then \
		update-mime-database "$(PREFIX)/share/mime"; \
	fi
	@if [ -z "$(DESTDIR)" ] && command -v update-desktop-database >/dev/null; then \
		update-desktop-database "$(PREFIX)/share/applications"; \
	fi
	@if [ -z "$(DESTDIR)" ] && command -v gtk-update-icon-cache >/dev/null; then \
		gtk-update-icon-cache -q -t -f "$(PREFIX)/share/icons/hicolor"; \
	fi

uninstall:
	rm -f $(BINDIR)/bbcat-gtk
	rm -f $(DATADIR)/applications/bbcat.desktop
	rm -f $(DATADIR)/applications/$(APP_ID).desktop
	rm -f $(DATADIR)/icons/hicolor/scalable/apps/$(APP_ID).svg
	rm -f $(DATADIR)/mime/packages/bbcat.xml
	@if [ -z "$(DESTDIR)" ] && command -v update-mime-database >/dev/null; then \
		update-mime-database "$(PREFIX)/share/mime"; \
	fi
	@if [ -z "$(DESTDIR)" ] && command -v update-desktop-database >/dev/null; then \
		update-desktop-database "$(PREFIX)/share/applications"; \
	fi
	@if [ -z "$(DESTDIR)" ] && command -v gtk-update-icon-cache >/dev/null; then \
		gtk-update-icon-cache -q -t -f "$(PREFIX)/share/icons/hicolor"; \
	fi
