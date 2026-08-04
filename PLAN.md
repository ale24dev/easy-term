# easy-term — Plan de Implementación

> Gestor de procesos de desarrollo para el menu bar de macOS, construido con Tauri v2.
> En lugar de tener N terminales abiertas con `pnpm run dev`, defines tus proyectos una vez
> y los levantas, paras y monitoreas desde un popover en la barra de menú.

---

## 1. Visión y alcance

**Qué es:** un *process manager* de proyectos dev. La terminal (PTY + render de logs) es el
medio, no el fin. El core es: `definir proyecto → levantarlo/pararlo → ver sus logs → conocer su estado`.

**Qué NO es (fuera de alcance consciente):**
- No es una terminal de propósito general con tabs de shell (para eso están Warp/iTerm).
- No es multiplataforma en v1 (tray positioning y PATH-fix son muy específicos de macOS; se abstrae después).
- Sin temas/customización visual hasta que el core sea sólido.

---

## 2. Stack técnico

| Capa | Tecnología | Motivo |
|---|---|---|
| Shell de la app | **Tauri v2** | Binario ligero, Rust en backend, acceso nativo a macOS |
| Menu bar | `TrayIcon` de Tauri + **`tauri-plugin-positioner`** (modo `TrayCenter`) | Ventana popover anclada al icono del tray |
| Procesos | **`portable-pty`** (crate Rust) | PTY real: los dev servers (Vite, Next) detectan TTY → colores ANSI, output correcto |
| Frontend | **React + TypeScript + Vite** | DX rápida, ecosistema |
| Render de logs | **xterm.js** (`@xterm/xterm` + addons `fit`, `search`, `web-links`) | Render ANSI fiel, búsqueda, links clicables |
| Estado UI | **Zustand** | Store simple para lista de proyectos y estados |
| Persistencia | JSON en `~/Library/Application Support/easy-term/projects.json` | Suficiente para v1; sin DB |
| Entorno | **`fix-path-env`** o resolución vía `$SHELL -lc` | Apps de menu bar no heredan el PATH del shell del usuario |
| Recursos | **`sysinfo`** (crate Rust) | CPU/RAM por proceso (fase 3) |

---

## 3. Modelo de datos

```typescript
interface Project {
  id: string;                 // uuid
  name: string;               // nombre visible
  path: string;               // ruta absoluta del proyecto
  command: string;            // ej. "pnpm run dev"
  port: number | null;        // puerto esperado (null = no aplica)
  env: Record<string, string>; // variables de entorno extra
  autoRestart: boolean;       // reiniciar si crashea (con backoff)
  groupId: string | null;     // workspace al que pertenece (fase 3)
}

type ProjectStatus = "stopped" | "starting" | "running" | "crashed";

interface ProjectRuntime {
  status: ProjectStatus;
  pid: number | null;
  detectedUrl: string | null; // URL real parseada de los logs (Vite puede cambiar de puerto)
  errorCount: number;         // errores desde la última vez que se vieron los logs
  startedAt: number | null;
}

interface Group {
  id: string;
  name: string;
  projectIds: string[];       // en orden de arranque
}
```

**Backend Rust** — estructura equivalente con `serde`. El estado runtime vive solo en memoria
(un `HashMap<ProjectId, ManagedProcess>` detrás de un `Mutex`/`RwLock` en el estado de Tauri);
la config persiste a JSON con escritura atómica (write-to-temp + rename).

---

## 4. Arquitectura

```
┌─────────────────────────── Frontend (WebView) ───────────────────────────┐
│  React + Zustand                                                         │
│  ├─ ProjectList (estados, acciones rápidas)                              │
│  ├─ ProjectForm (crear/editar, detección de scripts)                     │
│  └─ LogView (xterm.js, una instancia por proyecto, búsqueda, autoscroll) │
└───────────────┬──────────────────────────────▲───────────────────────────┘
                │  invoke (comandos)           │  emit (eventos)
┌───────────────▼──────────────────────────────┴───────────────────────────┐
│  Backend (Rust / Tauri)                                                  │
│  ├─ process_manager: spawn/kill/restart sobre portable-pty               │
│  │    └─ un reader-thread por proceso → emite chunks de output           │
│  ├─ project_store: CRUD + persistencia JSON atómica                      │
│  ├─ port_checker: ¿puerto ocupado? ¿por qué PID?                         │
│  ├─ script_detector: lee package.json + lockfiles                        │
│  ├─ env_resolver: PATH real vía login shell (una vez al arrancar)        │
│  └─ tray: icono con estado agregado + menú contextual                    │
└──────────────────────────────────────────────────────────────────────────┘
```

### Comandos Tauri (`invoke`)

| Comando | Firma | Descripción |
|---|---|---|
| `list_projects` | `() -> Vec<Project>` | Carga config persistida |
| `save_project` | `(Project) -> Project` | Crea o actualiza + persiste |
| `delete_project` | `(id) -> ()` | Para el proceso si corre, borra |
| `start_project` | `(id) -> Result<()>` | Spawn en PTY; error si puerto ocupado |
| `stop_project` | `(id) -> ()` | SIGTERM al process group; SIGKILL tras timeout |
| `restart_project` | `(id) -> ()` | stop + start |
| `write_stdin` | `(id, data) -> ()` | Input hacia el PTY (fase 4) |
| `detect_scripts` | `(path) -> DetectedConfig` | Scripts de package.json + package manager |
| `check_port` | `(port) -> PortStatus` | Libre / ocupado (con PID y nombre del proceso) |
| `kill_port_owner` | `(port) -> Result<()>` | Mata el proceso que ocupa el puerto |
| `get_process_stats` | `(id) -> Stats` | CPU/RAM (fase 3) |

### Eventos (backend → frontend)

| Evento | Payload | Cuándo |
|---|---|---|
| `process:output` | `{ id, chunk }` | Chunks de stdout/stderr del PTY (batched ~16ms) |
| `process:status` | `{ id, status, pid }` | Cambio de estado |
| `process:url-detected` | `{ id, url }` | Regex sobre logs detecta `http://localhost:*` |
| `process:exit` | `{ id, code }` | El proceso terminó (distingue exit 0 vs crash) |

### Decisiones técnicas clave

1. **PTY, no `Command::spawn` a secas.** Vite/Next/webpack detectan TTY. Sin PTY: sin colores,
   output bufferizado, spinners rotos. `portable-pty` con tamaño fijo inicial (200×50) y
   `resize` cuando cambie el tamaño del LogView.
2. **Kill del *process group*, no solo del PID.** `pnpm run dev` lanza hijos (node → vite →
   esbuild). Spawn con nuevo process group y `killpg(SIGTERM)` → esperar 3s → `SIGKILL`.
   Si no, quedan procesos zombis ocupando el puerto.
3. **El PATH problem.** Apps lanzadas desde Finder/menu bar reciben el PATH mínimo del sistema
   (`/usr/bin:/bin:...`) — no encuentran `pnpm`, `node` vía nvm/fnm, etc. Al arrancar la app,
   ejecutar una vez `$SHELL -lc 'printf %s "$PATH"'` (o usar `fix-path-env`) y usar ese
   entorno para todos los spawns.
4. **Backpressure de logs.** Un dev server verboso puede emitir MB/s. El reader-thread acumula
   en buffer y emite al frontend con throttle (~60fps máx). xterm.js mantiene scrollback
   limitado (10k líneas). El buffer Rust por proceso se limita (ring buffer ~1MB) para poder
   re-poblar el terminal al reabrir el popover.
5. **Instancias de xterm.js persistentes.** Una por proyecto, montada/desmontada del DOM al
   cambiar de proyecto pero nunca destruida mientras el proceso viva — así no se pierde el
   scrollback al navegar.
6. **Ventana popover:** `decorations: false`, `alwaysOnTop`, `skipTaskbar`, `hiddenTitle`,
   `ActivationPolicy::Accessory` (sin icono en el Dock). Se oculta on-blur.

---

## 5. Fases de implementación

### Fase 0 — Scaffold (½ día) ✅ completada
- [x] `pnpm create tauri-app` (React + TS + Vite), Tauri v2.
- [x] Configurar tray icon + `tauri-plugin-positioner`, ventana popover anclada, hide on blur.
- [x] `ActivationPolicy::Accessory` (solo menu bar, sin Dock).
- [x] CI básico: `cargo check` + `tsc --noEmit` + `cargo fmt --check` en GitHub Actions.

**Criterio de salida:** icono en el menu bar que abre/cierra un popover vacío.

### Fase 1 — Core MVP (3–5 días) ✅ completada
El objetivo: *usarla a diario reemplaza al menos una terminal*.

- [x] **1.1 Modelo + persistencia**: `project_store` con CRUD y JSON atómico
      (write-to-temp + rename) en `~/Library/Application Support/easy-term/projects.json`.
- [x] **1.2 Formulario de proyecto**: picker de carpeta nativo (`tauri-plugin-dialog`), nombre,
      comando, puerto, env vars (filas clave/valor).
- [x] **1.3 `env_resolver`**: PATH real del login shell, cacheado al arranque.
- [x] **1.4 `process_manager`**: start/stop/restart sobre `portable-pty`, process groups
      (`setsid` + `killpg` SIGTERM→SIGKILL con timeout de 3s), reader/batcher/waiter threads
      por proceso, eventos `process:status|output|exit|url-detected`, ring buffer de 1MB.
- [x] **1.5 LogView con xterm.js**: colores ANSI, autoscroll con pausa al scrollear arriba
      y botón "↓ seguir" para volver, fit addon, instancias persistentes por proyecto
      (`lib/terminals.ts`).
- [x] **1.6 Lista de proyectos**: estado con dot de color, botones start/stop/restart,
      click → ver logs.
- [x] **1.7 Estados y exits**: distinguir exit limpio vs crash; marcador de color en el
      propio log al terminar el proceso.
- [x] **1.8 `error_logger` (base)**: tipo `AppError` con códigos, writer JSONL con canal
      mpsc, panic hook, rotación/retención, dedupe de errores repetidos, captura global
      en frontend + comando `log_app_error` (ver sección 7).

**Criterio de salida:** ✅ verificado end-to-end (Linux, vía Xvfb + xdotool, ya que esta
sesión no tiene acceso a macOS): se crea un proyecto con el folder picker nativo, se persiste
en `projects.json`, `sh run.sh` corre en un PTY real con colores/output en vivo en xterm.js,
la URL `http://localhost:5173` se detecta, y al detener el proyecto el proceso se mata
limpiamente sin dejar zombis (`ps aux` confirmado). Cero eventos en el log de diagnóstico
durante la prueba. Pendiente de validar en macOS real: posicionamiento del popover respecto
al tray, `ActivationPolicy::Accessory`, y el picker de carpetas nativo (NSOpenPanel).

### Fase 2 — Los features que justifican la app (3–4 días) ✅ completada
- [x] **2.1 Detección automática** (`script_detector`): al escoger carpeta, parsear
      `package.json` → dropdown de scripts; package manager por lockfile
      (`pnpm-lock.yaml` → pnpm, `bun.lockb`/`bun.lock` → bun, `yarn.lock` → yarn,
      `package-lock.json` → npm). Pre-rellena nombre con el de `package.json`.
- [x] **2.2 Gestión de puertos** (`port_checker`): check pre-start vía `lsof`; si ocupado,
      dialog con el proceso dueño (nombre + PID) y opción "Liberar y continuar"
      (SIGTERM→SIGKILL). Resuelve `EADDRINUSE` en un click.
- [x] **2.3 Tray con estado**: título del tray con dot verde (🟢 N corriendo) / rojo
      (🔴 algo crasheó). Menú contextual con un item por proyecto (start/stop directo,
      glyph de estado) + Quit, se reconstruye reactivamente en cada cambio de estado o
      de lista de proyectos.
- [x] **2.4 Abrir en navegador**: botón en LogView con la URL detectada (o
      `http://localhost:{puerto}` como fallback) vía `process:url-detected`.
- [x] **2.5 Búsqueda en logs**: `@xterm/addon-search` con barra de búsqueda (Cmd/Ctrl+F),
      next/prev, Esc para cerrar.
- [x] **2.6 Notificaciones nativas de crash**: `tauri-plugin-notification`; el id del
      proyecto viaja en el campo `extra` de la notificación, un listener `onAction` en el
      frontend salta directo a los logs de ese proyecto al hacer click.
- [x] **2.7 Highlight de errores**: contador de líneas `error|warn` (regex sobre el
      stream) → badge numérico en `ProjectList`, se resetea al abrir los logs.
- [x] **2.8 Diagnóstico — dedupe y visor**: dedupe de errores repetidos ya en el
      `error_logger` base (Fase 1); panel Settings → Diagnóstico con visor de eventos,
      "Abrir carpeta de logs" y "Copiar último error".

**Validado end-to-end en Linux** (Xvfb + xdotool + dunst, sin acceso a macOS en esta
sesión): detección de scripts desde un `package.json` real (nombre y comando
pre-rellenados correctamente), diálogo de conflicto de puerto identificando el proceso
dueño real y liberándolo antes de arrancar, badge de errores incrementando en vivo con
reset al abrir logs, botón "Abrir en navegador" con la URL detectada, búsqueda en logs
resaltando coincidencias y haciendo scroll automático, panel de diagnóstico mostrando
"sin errores" tras toda la sesión de pruebas (cero errores internos logueados). Pendiente
de validar en macOS real: aspecto del ícono/título del tray nativo y el comportamiento
de click-through de las notificaciones (macOS puede diferir del mecanismo Linux/dbus
usado aquí para probar `onAction`).

**Criterio de salida:** configurar un proyecto nuevo son 2 clicks; un puerto ocupado se
resuelve desde la app; me entero de un crash sin mirar la app.

### Fase 3 — Power features (4–6 días) ✅ completada
- [x] **3.1 Grupos/workspaces**: entidad `Group` (persistida junto a `Project` en el mismo
      archivo), "levantar todo" en orden secuencial con espera de readiness (puerto abierto,
      poll cada 300ms, timeout de 8s; sin puerto → espera fija de 2s). UI: sección colapsable
      por grupo con botones start/stop grupal; el campo "Grupo" del formulario resuelve o
      crea el grupo por nombre.
- [x] **3.2 Auto-restart on crash**: checkbox `autoRestart` por proyecto; backoff exponencial
      (1s→2s→4s→8s→16s, cap 30s) con límite de 5 intentos; cancelación vía "epoch" al hacer
      stop explícito (manual, restart, o delete); reset del contador solo si el proceso
      sobrevivió ≥5s (ver nota de bug abajo). Badge "reintentando N/5" en la lista, con botón
      "✕ Cancelar" mientras hay un reintento pendiente.
- [x] **3.3 Atajo global** (`tauri-plugin-global-shortcut`): `Alt+Space` muestra/oculta el
      popover, mismo path que el click izquierdo del tray.
- [x] **3.4 Acciones rápidas por proyecto**: abrir en editor (`open_in_editor` prueba
      `cursor` y luego `code` vía PATH resuelto), abrir en Finder (`revealItemInDir`),
      copiar URL — los tres como botones en `LogView`.
- [x] **3.5 Monitor de recursos**: en vez de `sysinfo`, se optó por `ps -Ao pgid,pcpu,rss`
      sumado por process-group (mismo enfoque que `port_checker`/`lsof`, sin dependencias
      nuevas) — verificado que `pid == pgid` se cumple por el `setsid` de `portable-pty`.
      Polling cada 2s desde el frontend, activo solo mientras la ventana tiene foco
      (`onFocusChanged`).
- [x] **3.6 Launch at login** (`tauri-plugin-autostart`, toggle en Settings) + restaurar
      proyectos: flag `wasRunning` por proyecto, marcado al hacer Quit (snapshot de
      proyectos en `running`/`starting`) y consumido una sola vez al arrancar.

**Bug encontrado y corregido durante el testing E2E:** `start()` emitía `Running`
inmediatamente al spawnear el proceso (antes de saber si sobrevivía), y `emit_status`
reseteaba el contador de reintentos a 0 en cada `Running` — con un comando que crashea en
milisegundos, esto hacía que el contador nunca superara 1 y el backoff se quedara fijo en
~1s para siempre, sin llegar nunca al límite de 5 intentos (loop infinito). Se corrigió
reemplazando el reset-on-Running por un reset condicionado a que el proceso haya vivido al
menos `MIN_UPTIME_FOR_RESTART_RESET` (5s) antes de crashear — un proceso que crashea rápido
sigue acumulando en la misma racha de backoff; uno que corrió un buen rato antes de morir
empieza una racha nueva. Verificado con un proyecto que crashea al instante: deltas reales
entre crashes de 1.01s, 2.01s, 4.01s, 8.01s, 16.01s y luego `PROC_RESTART_LIMIT_REACHED`
una sola vez, sin más reintentos.

**Validado end-to-end en Linux** (Xvfb + xdotool): grupo de 2 proyectos con start-all
secuencial (readiness por puerto real vía `python3 -m http.server`) y stop-all concurrente
sin procesos huérfanos; auto-restart con backoff exponencial real hasta agotar los 5
intentos; notificación nativa de crash disparándose correctamente bajo Xvfb; cálculo de
CPU/RAM verificado contra el `ps` real del proceso corriendo. Pendiente de confirmar en
macOS real: el atajo global `Alt+Space` (no testeable de forma confiable vía X11 synthetic
events) y el comportamiento exacto de `tauri-plugin-autostart` (macOS usa LaunchAgents).

**Bug reportado en macOS real (post-Fase 3): click en el tray solo mostraba "Quit".**
El tray (2.3) adjuntaba un menú nativo vía `tray.set_menu()` para el status agregado y el
toggle rápido de proyectos. En macOS, `NSStatusItem.setMenu()` hace que AppKit muestre ese
menú en **todo** click —izquierdo incluido— sin importar `show_menu_on_left_click`; es un bug
conocido y no resuelto de Tauri (tauri-apps/tauri#4002), no reproducible en Linux porque el
backend de tray ahí es GTK, arquitectónicamente distinto. Con el menú adjunto, el click
izquierdo dejó de abrir el popover del todo. Corregido eliminando por completo el menú nativo
del tray: el título/tooltip del tray (`tray.rs`) sigue reflejando el estado agregado y
por-proyecto (glyphs `🟢/🟡/🔴/⚪` en el tooltip al hacer hover), pero "Salir" se movió a un
botón dentro de Settings y el toggle por-proyecto vive únicamente en la lista del popover.
Verificado en Linux (sin regresión en el código Rust compartido) y confirmado el flujo
completo del nuevo comando `quit_app` end-to-end (snapshot de `wasRunning` + `app.exit(0)`).

**Bug reportado en macOS real: el selector de carpeta se abría y se cerraba solo.**
Mismo patrón que el bug anterior, distinta puerta de entrada: el handler de `Focused(false)`
en `lib.rs` oculta la ventana del popover al perder foco (para que se comporte como un
popover normal). El picker de carpeta (`@tauri-apps/plugin-dialog`, botón "Elegir…" en
`ProjectForm`) se presenta en macOS como una *sheet* adjunta a esa ventana — abrirlo le quita
el foco a la ventana, disparando el hide, y ocultar la ventana se lleva puesta a su propia
sheet, que se cierra de inmediato. Corregido con un flag (`SuppressAutoHide`, `AtomicBool` en
el estado de la app) que el frontend activa justo antes de invocar `open()` y desactiva al
resolver la promesa (comandos `begin_native_dialog`/`end_native_dialog`); el handler de blur
respeta el flag y no oculta la ventana mientras el diálogo está abierto. Verificado en Linux
que el flujo IPC no rompe nada (el picker ahí es GTK vía portal, no reproduce el bug de sheet
en sí, pero confirma que no hay regresión): seleccionar carpeta completa `path`/`name`/
detección de scripts con normalidad. Pendiente confirmar en macOS real que el picker ya no se
cierra solo.

### Fase 3.7 — Suite de pruebas en capas

Hasta acá, todo el testing había sido manual (Xvfb + xdotool en el sandbox Linux, más
verificación puntual en macOS real para los dos bugs de arriba) — cero tests automatizados,
y el CI solo corría `tsc --noEmit` + `cargo fmt/check`. A raíz del segundo bug de macOS real
(picker de carpeta), se armó una suite en tres capas, cada una con un propósito distinto:

- [x] **Rust (`cargo test`, 41 tests)**: unitarios/integración para toda la lógica de negocio
      que no depende de un `AppHandle` real — `project_store` (CRUD, persistencia atómica,
      grupos, `wasRunning`), `script_detector` (detección de package manager, parseo de
      scripts), `port_checker` (contra un listener real vía `python3 -m http.server`, no
      mocks), `env_resolver` (resolución de PATH vía shell), y de `process_manager` el backoff
      exponencial + detección de URL + conteo de líneas de error (extraídos a funciones puras)
      más un smoke test directo sobre `portable-pty` que valida los supuestos de
      setsid/process-group que `start()`/`stop()` dan por sentado. `store_path` en
      `project_store` se inyectó (antes era un global fijo a `~/Library/Application Support`)
      para poder apuntar los tests a un tempdir sin tocar datos reales.
- [x] **Frontend (`pnpm test`, Vitest, 16 tests)**: el store de Zustand (`stores/projects.ts`)
      con `ipc` mockeado — altas/ediciones in-place, que ningún método propague un error de
      ipc, y que los setters de runtime mergeen en vez de reemplazar.
- [x] **macOS UI E2E (`e2e/macos/`, AppleScript vía System Events)**: la única capa que
      realmente ejercita AppKit — las dos capas anteriores corren en Linux y por diseño no
      hubieran detectado ninguno de los dos bugs reales de esta fase (ambos específicos de
      AppKit). `tauri-driver`/WebDriver, la herramienta que Tauri recomienda para E2E, **no
      soporta macOS** (solo Linux/Windows), así que esta capa maneja la app compilada
      directamente por accesibilidad: clic en el ícono del tray, click en botones por su texto
      accesible (`name`/`help`/`description`, ya que los botones de ícono usan `title=` en vez
      de texto visible), "Ir a la carpeta" (⌘⇧G) para el picker nativo. Cuatro flujos:
      `01_tray_toggle` y `02_folder_picker` son regresión directa de los dos bugs recién
      corregidos; `03_project_crud` es el happy path (crear/iniciar/detener/eliminar, con un
      `sleep 30` en vez de un dev server real para no depender de pnpm/red); `04_quit` valida
      que "Salir de easy-term" mata el proceso de verdad.

**Nota de confianza sobre la capa E2E de macOS**: se escribió sin acceso a una Mac real para
correrla — los selectores de accesibilidad siguen convenciones documentadas pero no están
verificados contra una corrida real. Por eso el job `macos-e2e` en CI está con
`continue-on-error: true`: corre en cada push/PR contra `macos-latest`, pero no bloquea el
merge todavía. Una vez que una corrida real confirme que los selectores funcionan (o se
ajusten los que no), sacar el `continue-on-error` para que sí bloquee. `check` (tsc, fmt,
clippy-equivalente, `cargo test`, `pnpm test`) sí es bloqueante desde ya — esa capa se corrió
y se verificó en este sandbox.

### Fase 3.8 — Rediseño de UI (Tailwind + shadcn/ui)

Pedido explícito de rediseñar toda la UI guiándose por [tauri-ui](https://github.com/agmmnn/tauri-ui)
(agmmnn), un scaffolder que combina Tauri con shadcn/ui (Radix UI + Tailwind + theming
claro/oscuro). tauri-ui en sí es un generador de proyectos Next.js/Vite nuevos, no algo
"instalable" dentro de una app ya existente — se tradujo el patrón (Tailwind + primitivas
Radix con la API de shadcn/ui + theming por variables CSS) a mano sobre el Vite+React ya
existente:

- [x] **Tailwind v4** (`@tailwindcss/vite`, CSS-first: `@theme inline` en `App.css`, sin
      `tailwind.config.js`) + alias `@/*` → `src/*` en `vite.config.ts`, `vitest.config.ts` y
      `tsconfig.json`.
- [x] **Primitivas al estilo shadcn/ui escritas a mano** (`src/components/ui/`): Button (cva,
      variants default/destructive/outline/secondary/ghost/link), Input, Label, Switch, Badge,
      Dialog, Select, Tooltip — sobre los primitivos de Radix UI (`@radix-ui/react-*`) más
      `class-variance-authority`/`clsx`/`tailwind-merge`/`lucide-react`. **El CLI de shadcn no
      se pudo usar**: su flujo `init`/`add` actual depende de una llamada de red a
      `ui.shadcn.com` para resolver el registry, y ese host está bloqueado por la política de
      red de este entorno (403 confirmado contra el proxy) — se instalaron los mismos paquetes
      Radix que el CLI habría instalado y se escribieron los componentes a mano siguiendo la
      API pública de shadcn/ui (mismos nombres de props/variantes), así que son
      intercambiables con los del CLI si se corre en un entorno sin esa restricción más
      adelante.
- [x] **Tokens de diseño** en `App.css`: paleta neutral de shadcn en OKLCH (`--background`,
      `--foreground`, `--card`, `--primary`, `--border`, `--radius`, etc.) para claro y oscuro,
      más tokens propios `--status-running/starting/crashed/stopped` (constantes entre temas:
      son señales de estado, no acentos decorativos).
      `@custom-variant dark (&:is(.dark *))` en vez de la estrategia por defecto de Tailwind
      (`prefers-color-scheme`), para poder tener "claro"/"oscuro"/"sistema" como opciones
      explícitas.
- [x] **`theme-provider.tsx`**: persiste la elección en `localStorage`, sigue
      `prefers-color-scheme` en vivo cuando el modo es "sistema", y aplica la clase `.dark`
      sincrónicamente en `main.tsx` antes del primer render (sin esperar al efecto) para evitar
      un flash del tema incorrecto. Toggle de tres estados (☀️/🌙/🖥️) en Settings.
- [x] **Los cinco componentes reescritos** sobre las primitivas nuevas: `App.tsx` (header con
      iconos de lucide-react en vez de glifos de texto), `ProjectList.tsx` (acciones que
      aparecen al hacer hover sobre la fila, badges para puerto/errores/reintentos),
      `ProjectForm.tsx` (Select de Radix para scripts detectados, Switch para auto-restart),
      `Settings.tsx` (toggle de tema, badges de nivel en diagnóstico), `PortConflictDialog.tsx`
      (Dialog de Radix con foco atrapado en vez del backdrop/modal hecho a mano). `App.css`
      quedó reducido a la configuración de Tailwind/tokens más lo genuinamente bespoke
      (`.terminal-host` para xterm.js, `.scrollbar-thin`).

**Cambio de texto accesible que afecta a `e2e/macos`**: el botón "+ Proyecto" pasó a tener un
ícono real (`PlusIcon`) en vez del carácter "+" como texto, así que su nombre accesible pasó
de "+ Proyecto" a "Proyecto" — se actualizaron los flujos `02_folder_picker.sh` y
`03_project_crud.sh` para que sigan apuntando al botón correcto. El resto de los selectores
(`Salir de easy-term`, `Elegir…`, labels de campos) no cambiaron de texto.

Verificado visualmente en Linux (Xvfb + xdotool): tema claro, tema oscuro (incluyendo el
tooltip de Radix confirmando que el toggle funciona), formulario completo con Switch/Select/
Input, picker de carpeta nativo (GTK) sin regresión, fila de proyecto con badge de estado y
acciones reveal-on-hover, crear/eliminar un proyecto de punta a punta. `cargo test` (41),
`pnpm test` (16) y `pnpm build` (incluye `tsc`) siguen en verde.

**Bug reportado en macOS real: la app no se podía abrir desde el tray con otra app en
pantalla completa.** Un `NSWindow` solo puede aparecer, por defecto, en el Space en el que se
mostró la última vez; `visible_on_all_workspaces` (que Tauri sí expone, pero que este proyecto
nunca usó) lo agregaría a todos los Spaces *normales* — pero un Space ocupado por una app en
pantalla completa es un Space exclusivo aparte, fuera de esa lista. Sin el flag
`NSWindowCollectionBehaviorFullScreenAuxiliary` (el que usan utilidades de la barra de menú
como Bartender/Ice/iStat Menus para poder mostrarse mientras otra app está en fullscreen), el
click en el tray no tenía ningún efecto visible: el popover intentaba abrirse en un Space que
el usuario no podía ver. Tauri/tao no exponen ese flag por su API pública, así que se agregó
`macos_window.rs`, que toma el puntero crudo al `NSWindow` vía `window.ns_window()` y le
suma `CanJoinAllSpaces | FullScreenAuxiliary` a su `collectionBehavior` directamente por
`objc2`/`objc2-app-kit` (agregados como dependencia solo para `cfg(target_os = "macos")`,
misma versión que ya traían tao/wry transitivamente — no se duplica ni diverge nada en
`Cargo.lock`). Se llama una sola vez en `setup()`, ya que `collectionBehavior` es una
propiedad persistente del `NSWindow`, no algo que se resetee entre `hide()`/`show()`.

**Nota de confianza**: no se pudo compilar ni un `cargo check` de este código — este sandbox
es Linux sin toolchain de cross-compilación a macOS (`objc2-exception-helper` necesita
compilar un `.m` con `-arch`/`-mmacosx-version-min`, que el `cc` de Linux no soporta), así que
un `cargo check --target x86_64-apple-darwin` falla en el build script antes de llegar a
tipar el código nuevo. La superficie de riesgo se redujo lo más posible: `objc2`/
`objc2-app-kit` ya son dependencias transitivas de tao/wry en las mismas versiones exactas
(0.6.4/0.3.2, confirmado en `Cargo.lock` — no hay una segunda copia divergente), y las firmas
de `NSWindow::collectionBehavior`/`setCollectionBehavior` y los bits de
`NSWindowCollectionBehavior` se verificaron a mano contra el código fuente del crate
instalado. Aun así, esto necesita confirmarse en una Mac real — no está probado.

### Fase 4 — Diferenciadores (backlog, priorizar según uso real)
- [ ] **4.1 Terminal interactiva**: `write_stdin` + `onData` de xterm.js → responder prompts
      del dev server ("port in use, use 3001? y/n"). Con el PTY ya montado es casi gratis.
- [ ] **4.2 Config por repo**: `easyterm.json` en la raíz del proyecto; al escoger la carpeta,
      si existe, pre-carga todo → config compartible con el equipo.
- [ ] **4.3 Integración git**: rama actual junto al nombre del proyecto (leer `.git/HEAD`,
      watch con debounce); opcional: prompt de restart al cambiar de rama.
- [ ] **4.4 Distribución**: firma + notarización de la app, updater de Tauri, tap de Homebrew.

---

## 6. Estructura de directorios propuesta

```
easy-term/
├── src/                        # Frontend React
│   ├── components/
│   │   ├── ProjectList.tsx
│   │   ├── ProjectForm.tsx
│   │   ├── LogView.tsx         # wrapper de xterm.js
│   │   └── PortConflictDialog.tsx
│   ├── stores/
│   │   └── projects.ts         # Zustand: proyectos + runtime status
│   ├── lib/
│   │   ├── ipc.ts              # wrappers tipados de invoke/listen
│   │   └── terminals.ts        # registry de instancias xterm persistentes
│   └── App.tsx
├── src-tauri/
│   ├── src/
│   │   ├── main.rs
│   │   ├── tray.rs
│   │   ├── commands.rs         # #[tauri::command] handlers
│   │   ├── process_manager.rs  # PTY, process groups, reader threads
│   │   ├── project_store.rs    # CRUD + persistencia JSON
│   │   ├── port_checker.rs
│   │   ├── script_detector.rs
│   │   ├── env_resolver.rs
│   │   └── error_logger.rs     # AppError, writer JSONL, panic hook, rotación
│   ├── Cargo.toml
│   └── tauri.conf.json
├── PLAN.md                     # este documento
└── package.json
```

---

## 7. Diagnóstico interno: registro de errores de la app

La app registra **sus propios errores** (no los de los proyectos del usuario) en logs
estructurados JSON, para poder diagnosticar y corregir fallos a posteriori sin depender
de que el usuario reproduzca el problema.

### 7.1 Formato y ubicación

- **Formato: JSONL** (un objeto JSON por línea, append-only). Más robusto que un array JSON:
  una línea corrupta no invalida el archivo, el append es atómico a nivel de línea y se
  procesa con streaming.
- **Ubicación:** `~/Library/Logs/easy-term/errors-YYYY-MM-DD.jsonl` (la convención de macOS
  para logs de apps; visible en Console.app).
- **Rotación:** un archivo por día; retención de 14 días + límite global de 20 MB
  (se borra lo más antiguo primero). Limpieza al arrancar la app.

### 7.2 Esquema del evento de error

```jsonc
{
  "ts": "2026-08-04T14:32:11.482Z",     // ISO 8601 UTC
  "level": "error",                      // "warn" | "error" | "fatal"
  "source": "backend",                   // "backend" | "frontend"
  "module": "process_manager",           // módulo que originó el error
  "code": "PTY_SPAWN_FAILED",            // código estable de la taxonomía (ver 7.3)
  "message": "Failed to spawn PTY: No such file or directory",
  "context": {                           // datos específicos del error (best-effort)
    "projectId": "a1b2c3",
    "command": "pnpm run dev",
    "os_error": 2
  },
  "stack": "...",                        // stacktrace si existe (Rust backtrace o JS stack)
  "session": "f47ac10b",                 // id aleatorio por arranque de la app (agrupa errores de una sesión)
  "appVersion": "0.1.0",
  "osVersion": "macOS 15.2"
}
```

**Privacidad:** los logs son locales, nunca se envían a ningún sitio. Aun así, en `context`
se registran rutas de proyecto tal cual (es la máquina del usuario), pero **nunca** valores
de variables de entorno — solo sus nombres.

### 7.3 Taxonomía de códigos de error

Códigos estables (enum en Rust + union type en TS) para poder agrupar y buscar. Familias:

| Familia | Ejemplos | Módulo típico |
|---|---|---|
| `PTY_*` | `PTY_SPAWN_FAILED`, `PTY_RESIZE_FAILED`, `PTY_READ_ERROR` | `process_manager` |
| `PROC_*` | `PROC_KILL_FAILED`, `PROC_GROUP_ORPHANED`, `PROC_UNEXPECTED_EXIT` | `process_manager` |
| `STORE_*` | `STORE_READ_FAILED`, `STORE_WRITE_FAILED`, `STORE_PARSE_ERROR`, `STORE_MIGRATION_FAILED` | `project_store` |
| `ENV_*` | `ENV_SHELL_RESOLVE_FAILED`, `ENV_PATH_EMPTY` | `env_resolver` |
| `PORT_*` | `PORT_CHECK_FAILED`, `PORT_KILL_OWNER_FAILED` | `port_checker` |
| `DETECT_*` | `DETECT_PKG_JSON_INVALID`, `DETECT_IO_ERROR` | `script_detector` |
| `IPC_*` | `IPC_COMMAND_PANIC`, `IPC_EVENT_EMIT_FAILED` | `commands` |
| `UI_*` | `UI_UNHANDLED_ERROR`, `UI_UNHANDLED_REJECTION`, `UI_XTERM_ERROR`, `UI_RENDER_ERROR` | frontend |
| `TRAY_*` | `TRAY_UPDATE_FAILED`, `TRAY_POSITION_FAILED` | `tray` |

### 7.4 Arquitectura de captura

```
Frontend                                Backend (Rust)
────────                                ──────────────
window.onerror ──────┐
unhandledrejection ──┤                  error_logger (módulo central)
ErrorBoundary React ─┼─ invoke ───────▶  ├─ canal mpsc → writer thread único
try/catch en ipc.ts ─┘  log_app_error    ├─ serializa a JSONL + append
                                         ├─ rotación/retención
Rust: panic hook ───────────────────────▶├─ dedupe: mismo (code+message) > 10/min
Rust: Result<_, AppError> en comandos ──▶│         se colapsa en un evento "repeated"
tracing::error! (opcional, puente) ─────▶└─ fallback: eprintln! si el disco falla
```

Puntos de captura:

1. **Backend — tipo `AppError` central**: todos los comandos Tauri devuelven
   `Result<T, AppError>`; `AppError` porta `code`, `message`, `context` y se loguea
   automáticamente en el punto de conversión (impl de `From`/middleware), no en cada
   call-site. Así ningún error de comando escapa sin registrarse.
2. **Backend — panic hook**: `std::panic::set_hook` captura panics con backtrace
   (`level: "fatal"`) y hace flush antes de morir.
3. **Frontend — captura global**: `window.onerror` + `window.onunhandledrejection` +
   un `ErrorBoundary` de React envían al comando `log_app_error`. El wrapper `ipc.ts`
   también loguea todo `invoke` rechazado (con el nombre del comando en `context`).
4. **Writer único con canal**: los productores no tocan el disco; empujan a un canal
   `mpsc` y un thread dedicado escribe. Sin locks en el hot path, sin bloquear la UI,
   y el orden de eventos queda serializado.
5. **Anti-tormenta**: dedupe por `(code, message)` con ventana de 1 min (a partir de 10
   repeticiones se emite un solo evento con `"repeats": N`). Evita que un error en loop
   (p. ej. `PTY_READ_ERROR` por segundo) queme disco.

### 7.5 Comandos Tauri adicionales

| Comando | Firma | Descripción |
|---|---|---|
| `log_app_error` | `(FrontendError) -> ()` | Punto de entrada de errores del frontend |
| `read_error_log` | `(day?, limit?) -> Vec<ErrorEvent>` | Lee eventos para el visor interno |
| `open_logs_folder` | `() -> ()` | Abre `~/Library/Logs/easy-term/` en Finder |

### 7.6 UI mínima (Settings → Diagnóstico)

- Contador de errores de la sesión actual; si > 0, dot discreto en Settings.
- Visor simple: tabla de eventos (hora, code, message) con filtro por nivel; click →
  JSON completo expandido.
- Botones: "Abrir carpeta de logs" y "Copiar último error" (para pegarlo en un issue).

### 7.7 Futuro (fuera de v1)

- Botón "Reportar" que pre-rellena un issue de GitHub con el JSON del error.
- Envío opt-in de crash reports (nunca por defecto).
- Puente `tracing` → error_logger para correlacionar errores con logs de debug.

---

## 8. Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
|---|---|---|
| PATH incompleto al spawn (nvm/fnm/asdf) | Los proyectos no arrancan — bug #1 de esta clase de apps | `env_resolver` vía login shell desde Fase 1; test manual con node instalado solo vía nvm |
| Procesos zombis tras stop | Puertos ocupados, confusión | Process groups + SIGTERM→SIGKILL; test explícito con `lsof` en el criterio de salida de Fase 1 |
| Logs de alto volumen congelan la UI | App inusable con proyectos verbosos | Throttle de eventos + ring buffer en Rust + scrollback limitado en xterm |
| Popover pierde foco/posición en multi-monitor | UX rota | `tauri-plugin-positioner` + QA en setup de 2 monitores |
| App se cierra con procesos corriendo | Dev servers huérfanos | Handler de exit que hace killpg de todos los hijos; opción futura "keep running on quit" |

---

## 9. Definición de éxito de la v1

Al final de la Fase 2, la app debe pasar esta prueba de fuego:

> Durante una semana de trabajo normal, **no abro ninguna terminal para levantar/parar
> mis proyectos**. Todo — arrancar, ver por qué falló, liberar un puerto, abrir el
> localhost — sucede desde el menu bar.
