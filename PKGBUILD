# Maintainer: psyclyx
pkgname=kexec-menu
pkgver=0.6.0
pkgrel=1
pkgdesc='Filesystem-agnostic kexec boot menu'
arch=('x86_64' 'aarch64')
url='https://github.com/psyclyx/kexec-menu'
license=('MIT')
makedepends=('rust' 'musl')
source=("$pkgname-$pkgver.tar.gz::$url/archive/v$pkgver.tar.gz")
sha256sums=('SKIP')

build() {
  cd "$pkgname-$pkgver"

  if [[ "$CARCH" == "x86_64" ]]; then
    target=x86_64-unknown-linux-musl
  else
    target=aarch64-unknown-linux-musl
  fi

  rustup target add "$target" 2>/dev/null || true
  RUSTFLAGS='-C target-feature=+crt-static' \
    cargo build --release --target "$target"
}

package() {
  cd "$pkgname-$pkgver"

  if [[ "$CARCH" == "x86_64" ]]; then
    target=x86_64-unknown-linux-musl
  else
    target=aarch64-unknown-linux-musl
  fi

  install -Dm755 "target/$target/release/kexec-menu" "$pkgdir/usr/bin/kexec-menu"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
