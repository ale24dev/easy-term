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

### Fase 3 — Power features (4–6 días)
- [ ] **3.1 Grupos/workspaces**: entidad `Group`, "levantar todo" en orden secuencial con
      espera simple entre arranques (readiness: puerto abierto o timeout). UI: sección
      colapsable por grupo con botón start/stop grupal.
- [ ] **3.2 Auto-restart on crash**: opt-in por proyecto; backoff exponencial (1s, 2s, 4s...
      máx 30s) y límite de N intentos para no ciclar; badge "restarting (2/5)".
- [ ] **3.3 Atajo global** (`tauri-plugin-global-shortcut`): `⌥+Space` (configurable)
      muestra/oculta el popover, estilo Raycast.
- [ ] **3.4 Acciones rápidas por proyecto**: abrir en VS Code/Cursor (`code .`/`cursor .`
      con detección de cuál hay instalado), abrir en Finder, copiar URL.
- [ ] **3.5 Monitor de recursos**: CPU/RAM por process-tree vía `sysinfo`, polling 2s solo
      con el popover abierto; mini sparkline o texto junto a cada proyecto.
- [ ] **3.6 Launch at login** (`tauri-plugin-autostart`) + restaurar proyectos que estaban
      corriendo al cerrar (flag `wasRunning` persistido).

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
