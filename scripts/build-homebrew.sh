#!/usr/bin/env bash
# Compila, firma y notariza easy-term para distribuirla vía Homebrew Cask /
# descarga directa desde GitHub Releases.
#
# `tauri build` firma, notariza y "staplea" (adjunta el ticket de
# notarización) la app y el .dmg automáticamente en cuanto detecta las
# variables de entorno de Apple en el entorno — sin pasos manuales.
#
# Requisitos (una sola vez):
#   1. Apple Developer Program activo.
#   2. Un certificado "Developer ID Application" instalado en el llavero
#      (Xcode → Settings → Accounts → Manage Certificates → + →
#      "Developer ID Application"; si no aparece esa opción, comprueba en
#      developer.apple.com/account que la membresía del Program esté activa).
#   3. Una contraseña específica de aplicación para notarizar (NO la
#      contraseña normal de Apple ID): genera una en
#      https://appleid.apple.com/account/manage → Inicio de sesión y
#      seguridad → Contraseñas específicas de app.
#
# Uso:
#   export APPLE_ID="tu-apple-id@icloud.com"
#   export APPLE_PASSWORD="xxxx-xxxx-xxxx-xxxx"   # la contraseña de app, no la normal
#   ./scripts/build-homebrew.sh
#
# APPLE_TEAM_ID y APPLE_SIGNING_IDENTITY tienen valores por defecto /
# autodetección; se pueden sobreescribir por entorno si hace falta.
#
# El resultado es un .dmg firmado, notarizado y con el ticket adjunto en
# src-tauri/target/universal-apple-darwin/release/bundle/dmg/, listo para
# subir como asset de una GitHub Release y referenciar desde un Cask.

set -euo pipefail
cd "$(dirname "$0")/.."

APPLE_TEAM_ID="${APPLE_TEAM_ID:-9FU3PGG489}"

if [[ -z "${APPLE_ID:-}" || -z "${APPLE_PASSWORD:-}" ]]; then
  echo "error: faltan APPLE_ID y/o APPLE_PASSWORD (contraseña específica de app)." >&2
  echo "  Genera la contraseña en https://appleid.apple.com/account/manage" >&2
  echo "  → Inicio de sesión y seguridad → Contraseñas específicas de app." >&2
  echo "  export APPLE_ID=\"tu-apple-id@icloud.com\"" >&2
  echo "  export APPLE_PASSWORD=\"xxxx-xxxx-xxxx-xxxx\"" >&2
  exit 1
fi

# Busca en el llavero un certificado "Developer ID Application" del equipo.
# Es un certificado DISTINTO de "Apple Distribution" (ese solo vale para la
# App Store): fuera de la Store, Gatekeeper exige Developer ID.
if [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  for policy in basic codesigning; do
    APPLE_SIGNING_IDENTITY=$(security find-identity -v -p "$policy" 2>/dev/null |
      sed -n "s/.*\"\(Developer ID Application: [^\"]*(${APPLE_TEAM_ID})\)\".*/\1/p" | head -1) || true
    [[ -n "$APPLE_SIGNING_IDENTITY" ]] && break
  done
fi
if [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  echo "error: no se encontró el certificado 'Developer ID Application' del equipo $APPLE_TEAM_ID en el llavero." >&2
  echo "  Créalo en Xcode → Settings → Accounts → Manage Certificates → +" >&2
  echo "  → 'Developer ID Application'. Es un certificado distinto de los de" >&2
  echo "  la App Store (Apple Distribution / Mac Installer Distribution)." >&2
  echo "  Certificados disponibles: security find-identity -v -p basic" >&2
  exit 1
fi

IDENTIFIER=$(node -p "require('./src-tauri/tauri.conf.json').identifier")
PRODUCT_NAME=$(node -p "require('./src-tauri/tauri.conf.json').productName")
VERSION=$(node -p "require('./src-tauri/tauri.conf.json').version")

echo "==> App:      $PRODUCT_NAME $VERSION ($IDENTIFIER)"
echo "==> Team:     $APPLE_TEAM_ID"
echo "==> Firma:    $APPLE_SIGNING_IDENTITY"
echo "==> Apple ID: $APPLE_ID"

echo "==> Asegurando targets de Rust (binario universal)"
rustup target add aarch64-apple-darwin x86_64-apple-darwin

echo "==> Compilando, firmando y notarizando (tauri build)"
# Con APPLE_SIGNING_IDENTITY + APPLE_ID + APPLE_PASSWORD + APPLE_TEAM_ID en
# el entorno, tauri build firma la .app, la manda a notarizar a Apple,
# espera el resultado y adjunta el ticket (staple) sin pasos manuales.
export APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID
pnpm tauri build --target universal-apple-darwin

APP="src-tauri/target/universal-apple-darwin/release/bundle/macos/$PRODUCT_NAME.app"
DMG="src-tauri/target/universal-apple-darwin/release/bundle/dmg/${PRODUCT_NAME}_${VERSION}_universal.dmg"

if [[ ! -d "$APP" || ! -f "$DMG" ]]; then
  echo "error: no se encontraron los artefactos esperados ($APP / $DMG)." >&2
  exit 1
fi

echo "==> Verificando que el ticket de notarización quedó adjunto"
if xcrun stapler validate "$APP" 2>&1 | tee /tmp/easy-term-staple.log | grep -q "The validate action worked"; then
  echo "    ✅ Ticket adjunto correctamente."
else
  echo "    ⚠️  No se pudo confirmar el staple; revisa /tmp/easy-term-staple.log" >&2
fi

echo "==> Verificando que Gatekeeper acepta la app"
spctl -a -vvv --type execute "$APP" || {
  echo "error: Gatekeeper no acepta la app. Revisa la salida de arriba." >&2
  exit 1
}

SHA256=$(shasum -a 256 "$DMG" | cut -d' ' -f1)

echo
echo "✅ Listo: $DMG"
echo "   sha256: $SHA256"
echo
echo "Siguiente paso — publicar:"
echo "  1. Crea un GitHub Release con el tag v$VERSION en el repo de easy-term"
echo "     y sube \"$DMG\" como asset (debe quedar descargable SIN iniciar"
echo "     sesión: si el repo es privado, Homebrew no podrá bajarlo)."
echo "  2. Actualiza homebrew/easy-term.rb.in con esta versión y este sha256"
echo "     (o copia homebrew/easy-term.rb.in a tu tap y rellénalo)."
echo "  3. En tu tap personal (p. ej. github.com/ale24dev/homebrew-easy-term),"
echo "     coloca el Cask relleno en Casks/easy-term.rb y haz commit/push."
echo "  4. Instalar: brew tap ale24dev/easy-term && brew install --cask easy-term"
