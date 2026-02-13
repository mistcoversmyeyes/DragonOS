ARCH ?= x86_64
curl_version := 8.5.0
curl_tarball := curl-$(curl_version).tar.xz
curl_tarball_path := $(curl_tarball)
build_dir := build/$(ARCH)
curl_dir := $(build_dir)/curl-$(curl_version)
host := $(ARCH)-linux-musl
prefix := $(host)-
bin := $(build_dir)/curl

cc := $(prefix)gcc
strip := $(prefix)strip
ar := $(prefix)ar
ranlib := $(prefix)ranlib
curl_url_primary := https://mirrors.dragonos.org.cn/pub/third_party/curl/$(curl_tarball)
curl_url_fallback := https://curl.se/download/$(curl_tarball)

configure_args := \
	--host=$(host) \
	--disable-shared \
	--enable-static \
	--disable-manual \
	--disable-ldap \
	--disable-ldaps \
	--disable-rtsp \
	--disable-dict \
	--disable-file \
	--disable-ftp \
	--disable-gopher \
	--disable-imap \
	--disable-mqtt \
	--disable-pop3 \
	--disable-smb \
	--disable-smtp \
	--disable-telnet \
	--disable-tftp \
	--disable-websockets \
	--without-ssl \
	--without-openssl \
	--without-gnutls \
	--without-mbedtls \
	--without-rustls \
	--without-bearssl \
	--without-wolfssl \
	--without-libpsl \
	--without-brotli \
	--without-zstd \
	--without-zlib \
	--without-ca-bundle \
	--without-ca-path

$(curl_tarball_path):
	@if ! wget "$(curl_url_primary)"; then \
		rm -f "$(curl_tarball_path)"; \
		wget "$(curl_url_fallback)"; \
	fi

$(curl_dir): $(curl_tarball_path)
	mkdir -p $(build_dir)
	tar -xJf $< -C $(build_dir)

$(bin): $(curl_dir)
	cd $(curl_dir) && \
	PKG_CONFIG=/bin/false \
	CC="$(cc)" AR="$(ar)" RANLIB="$(ranlib)" STRIP="$(strip)" \
	CFLAGS="-Os -static" LDFLAGS="-static" \
	./configure $(configure_args)
	cd $(curl_dir) && \
	$(MAKE) CURL_LDFLAGS_BIN="-all-static" -j$(nproc)
	mkdir -p $(dir $(bin))
	cp $(curl_dir)/src/curl $(bin)
	$(strip) $(bin)

.PHONY: all install clean distclean

all: $(bin)

install: all
	cp $(bin) $(DADK_CURRENT_BUILD_DIR)/curl

clean:
	rm -rf build/

distclean: clean
	rm -f $(curl_tarball_path)
