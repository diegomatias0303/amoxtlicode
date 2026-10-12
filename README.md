# 📜 AmoxliCode IDE

> **AmoxliCode** (del náhuatl *Amoxtli* — libro/códice del conocimiento — y *Code*) es un entorno de desarrollo integrado (IDE) ultraligero y de alto rendimiento escrito en **Rust**, acelerado por **GPU (WGPU)** y diseñado para combinar la velocidad y el bajísimo consumo de memoria de Notepad++ (~15–30 MB RAM) con la estética moderna, aceleración gráfica y capacidades inteligentes de un editor contemporáneo.

---

## 🤖 Protocolo para IA / Agente de Código (Continuidad de Contexto)

> **IMPORTANTE PARA AGENTES IA:** Si este chat se reinicia, se compacta o se abre en una nueva sesión, **este README es tu fuente de verdad**. Léelo para entender inmediatamente la arquitectura, el estado actual, las reglas del código y continuar trabajando sin perder el rumbo.

### 📌 Modelo Mental del Código
1. **Pipeline de Renderizado 2D por GPU:**
   - AmoxliCode **no usa Electron ni WebView**. Es un motor gráfico 2D nativo con `winit` y `wgpu`.
   - Bucle a 60 FPS bloqueado con V-Sync (`PresentMode::Fifo`).
   - Capa 0: Fondo dinámico (Video MP4/GIF vía FFmpeg o gradientes interactivos RGB).
   - Capa 1: Rectángulos y paneles UI dibujados mediante un pipeline de quads de vértices sólidos (`src/solid_quad.wgsl`).
   - Capa 2: Texto renderizado con `cosmic-text` y `glyphon` con caché de glifos en GPU.
2. **Reglas de Oro de Arquitectura (¡NO ROMPER!):**
   - **Nunca bloquear el bucle principal:** Todo trabajo pesado (FFmpeg, compilación, terminal, procesos) corre en hilos separados con canales `crossbeam-channel` o `std::sync::mpsc`.
   - **Aislamiento de clics en la UI:** Cualquier nuevo panel, ventana flotante o dock debe estar excluido en `is_in_text_area()` de [`src/editor.rs`](file:///D:/herramientas/amoxtlicode/amoxtlicode/src/editor.rs) para evitar que los clics del ratón muevan el cursor o seleccionen texto por detrás.
   - **Scissoring de texto con cosmic-text:** Para evitar que el texto se dibuje fuera de su panel (bleed), usa rebanado de líneas (`lines[start..end].join("\n")`) y `bounds: TextBounds` en el `TextArea`.
   - **Manejo de procesos en Windows:** Al lanzar procesos (`Command`), usar siempre `.creation_flags(0x0800_0000)` (`CREATE_NO_WINDOW`) para evitar popups negros de consola. Al terminarlos, invocar `taskkill /F /T /PID` para eliminar todo el árbol de procesos hijos.
3. **Comandos de Verificación Inmediata:**
   ```powershell
   cargo check        # Validación rápida de tipos y préstamos
   cargo build        # Compilación completa del binario de desarrollo
   cargo run          # Ejecución de prueba
   ```

---

## 📂 Mapa de Módulos del Proyecto (`src/`)

| Archivo | Responsabilidad Principal |
| :--- | :--- |
| [`src/main.rs`](file:///D:/herramientas/amoxtlicode/amoxtlicode/src/main.rs) | Bucle de eventos `winit`, inicialización `wgpu`, render pass unificado, despacho de eventos de ratón/teclado y atajos globales. |
| [`src/editor.rs`](file:///D:/herramientas/amoxtlicode/amoxtlicode/src/editor.rs) | Núcleo del editor: buffers de texto, tabs, cursor, selección, layout geométrico del dock, scroll y preparación para Glyphon. |
| [`src/terminal.rs`](file:///D:/herramientas/amoxtlicode/amoxtlicode/src/terminal.rs) | Panel dock inferior: Runner directo (NetBeans/VS style), gestión de procesos en background, streaming stdout/stderr, entrada stdin interactiva y shell. |
| [`src/syntax.rs`](file:///D:/herramientas/amoxtlicode/amoxtlicode/src/syntax.rs) | Resaltado sintáctico AST mediante Tree-Sitter (C, Java, JS, HTML, CSS, SQL, Rust, etc.). |
| [`src/settings.rs`](file:///D:/herramientas/amoxtlicode/amoxtlicode/src/settings.rs) | Menú de configuración interactivo: temas, paletas personalizadas, opacidades y modo RGB gamer. |
| [`src/lsp.rs`](file:///D:/herramientas/amoxtlicode/amoxtlicode/src/lsp.rs) | Cliente y protocolo LSP (Language Server Protocol) y popup de autocompletado flotante. |
| [`src/gradient.rs`](file:///D:/herramientas/amoxtlicode/amoxtlicode/src/gradient.rs) | Pipeline y buffers de gradientes y efectos cromáticos en GPU. |
| [`src/video.rs`](file:///D:/herramientas/amoxtlicode/amoxtlicode/src/video.rs) | Decodificación y carga de cuadros de video en segundo plano vía FFmpeg. |
| [`src/ui.rs`](file:///D:/herramientas/amoxtlicode/amoxtlicode/src/ui.rs) | Detección de clics en menús y cajas delimitadoras. |
| [`src/color.rs`](file:///D:/herramientas/amoxtlicode/amoxtlicode/src/color.rs) | Utilidades de conversión y manipulación de color (RGBA, Hex). |
| [`src/audio.rs`](file:///D:/herramientas/amoxtlicode/amoxtlicode/src/audio.rs) | Efectos de sonido y retroalimentación auditiva mecánica al teclear. |

---

## ✨ Características y Estado Actual

- [x] **Motor de edición veloz:** Edición de texto con selección por ratón y teclado, portapapeles (`Ctrl+C`, `Ctrl+V`, `Ctrl+X`), deshacer/rehacer (`Ctrl+Z`, `Ctrl+Y`).
- [x] **Panel de Ejecución Integrado (NetBeans / Visual Studio style):**
  - **Pestaña `▶ Salida`:** Guarda y ejecuta el archivo actual al presionar `F5` o el botón `[▶ Ejecutar]`. Muestra el comando exacto, directorio, salida en tiempo real en una línea limpia (sin mezclar texto con separadores), tiempo transcurrido y código de salida (`0 OK` o error).
  - **Botones de cabecera alineados píxel-perfect:** `[Ejecutar]`, `[Detener]`, `[Limpiar]`, `[-]`, `[100%]`, `[+]`, `[✕]` perfectamente centrados en sus cajas de color individuales con respuesta táctil y resaltado al pasar el ratón (*hover*).
  - **Entrada estándar interactiva (`stdin`):** Permite escribir respuestas para programas que requieren entrada (`input()`, `scanf()`) con historial de comandos (`↑` / `↓`).
  - **Botón `[Detener]`:** Detención limpia del proceso y subprocesos hijos (`Shift+F5` o `Ctrl+C`).
  - **Pestaña `💻 Terminal`:** Terminal interactiva PowerShell/CMD en segundo plano.
  - **Redimensión interactiva:** Barra superior arrastrable para ajustar la altura del dock.
  - **Aislamiento de clics:** Clics en la terminal no tocan el código del editor.
- [x] **Detección inteligente de compiladores e intérpretes:**
  - Auto-detección en Windows/Linux de **GCC**, **Clang (LLVM)** y **MSVC (cl)** para C y C++. Si no tienes GCC pero tienes Clang, AmoxliCode compila automáticamente con Clang sin errores ni configuraciones manuales.
  - Auto-detección de intérpretes de Python (`python`, `py`, `python3`) y soporte para Rust (`cargo run` con proyecto o `rustc` para archivos sueltos).
  - Mensajes de error amigables y accionables si no hay un compilador instalado en el sistema.
- [x] **Zoom personalizable e independiente para la Terminal:**
  - **`Ctrl + Rueda del ratón`** sobre la terminal para acercar o alejar el texto (60% a 250%).
  - **Botones dedicados en el dock:** `[-]`, indicador `100%` (clic para resetear) y `[+]`.
  - **Atajos de teclado:** `Ctrl + +` / `Ctrl + -` / `Ctrl + 0` con la terminal enfocada.
  - **Persistencia automática:** El nivel de zoom se guarda en `amoxlicode_settings.json` y se restaura entre sesiones.
- [x] **Pestañas de múltiples documentos:** Múltiples pestañas en la barra superior con títulos, indicador de modificaciones pendientes (`*`), botón de cierre rápido `×` y botón `+` para añadir pestañas.
- [x] **Apertura y creación de archivos desde la consola (CMD / PowerShell / Terminal):**
  - Permite invocar `amoxlicode archivo.html` (o múltiples archivos) directamente desde CMD o PowerShell.
  - **Comportamiento idéntico a Notepad++:** Si el archivo no existe en el disco, muestra el diálogo nativo: *"El archivo 'archivo.html' no existe. ¿Deseas crearlo?"*. Al presionar [Sí], el archivo se genera en el directorio de trabajo y se abre inmediatamente en el editor con su sintaxis asignada automáticamente (HTML, C, Python, etc.).
  - Soporta banderas de creación directa (`-n`, `--new`, `-c`, `--create`) para crear sin confirmación.
  - Reemplaza la pestaña virgen inicial para que el archivo especificado sea el único abierto al arrancar.
- [x] **Resaltado de sintaxis:** Árbol sintáctico con Tree-sitter para C, Java, JavaScript, HTML, CSS, SQL, Python, Rust.
- [x] **Autocompletado flotante:** Popup IntelliSense estilo LSP sobre el cursor.
- [x] **Personalización visual:** Fondos de video (.mp4/gif) o degradados animados, sliders de opacidad independientes y modo RGB.
- [x] **Sonido de tecleo:** Efectos de audio mecánicos/personalizados configurables.

---

## 🔄 Metodología para Nuevas Etapas del Proyecto

Para mantener la robustez, limpieza y alto rendimiento del proyecto, cualquier nueva etapa debe seguir este ciclo de 5 pasos:

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│ 1. Especificar  │ ──> │ 2. Diseñar      │ ──> │ 3. Implementar  │
│ Requisitos & UI │     │ Estado & Canales│     │ Módulos Limpios │
└─────────────────┘     └─────────────────┘     └─────────────────┘
                                                         │
                                                         ▼
┌─────────────────┐                             ┌─────────────────┐
│ 5. Documentar & │ <────────────────────────── │ 4. Verificar    │
│ Actualizar RM   │                             │ Check, Build & T│
└─────────────────┘                             └─────────────────┘
```

1. **Fase 1: Especificación y Contratos de UI:**
   - Definir qué problema resuelve la nueva característica.
   - Determinar las coordenadas visuales y si afecta el viewport de texto (`set_terminal_height`, márgenes laterales, etc.).
   - Diseñar atajos de teclado compatibles con los existentes.
2. **Fase 2: Modelado de Estado y Concurrencia:**
   - Mantener las estructuras de datos separadas por módulo.
   - Si requiere E/S o procesos, usar canales desacoplados (`crossbeam-channel`).
3. **Fase 3: Implementación Incremental:**
   - Modificar primero las estructuras de datos y métodos auxiliares.
   - Integrar la detección de eventos en `src/main.rs`.
   - Ajustar el layout y render pass en `src/editor.rs` o módulos dedicados.
4. **Fase 4: Verificación Rigurosa:**
   - Ejecutar `cargo check` para asegurar cero advertencias de préstamos o tipos.
   - Ejecutar `cargo build` para verificar generación limpia del binario.
   - Probar casos de borde (ej. redimensionado de ventana, clics fuera de límites, archivos grandes).
5. **Fase 5: Actualización del README:**
   - Marcar la etapa como completada en la hoja de ruta y documentar los nuevos controles.

---

## 🗺️ Hoja de Ruta (Roadmap de Nuevas Etapas)

### 📌 Etapa 1: Explorador de Archivos Lateral (File Tree Sidebar)
- [ ] Panel lateral colapsable a la izquierda (`Ctrl+B`).
- [ ] Vista en árbol de carpetas y archivos del proyecto abierto.
- [ ] Clic para abrir archivos en el editor.
- [ ] Iconos según la extensión del archivo.
- [ ] Menú contextual básico (Crear archivo, renombrar, eliminar).

### 📌 Etapa 2: Pestañas de Múltiples Archivos (Multi-Buffer Tabs) [COMPLETADA]
- [x] Barra superior de pestañas con nombres de archivo.
- [x] Indicador visual de archivo modificado (`●`).
- [x] Cierre individual por pestaña (`×`) y creación con `+`.
- [x] Persistencia y cambio dinámico de lenguaje por archivo.

### 📌 Etapa 3: Búsqueda y Reemplazo Global (Find & Replace)
- [ ] Barra flotante de búsqueda (`Ctrl+F`).
- [ ] Resaltado de todas las coincidencias en el buffer activo.
- [ ] Búsqueda y reemplazo en archivo (`Ctrl+H`).
- [ ] Opciones: Coincidir mayúsculas/minúsculas, palabra completa y expresiones regulares.

### 📌 Etapa 4: Indicadores de Git en el Editor
- [ ] Detección automática del repositorio Git activo.
- [ ] Visualización de la rama actual en la barra de estado.
- [ ] Barras de color en el margen izquierdo (gutter) para líneas añadidas (+), modificadas (~) o eliminadas (-).

### 📌 Etapa 5: Depuración Visual Básica (DAP / Breakpoints)
- [ ] Margen clickeable para colocar puntos de interrupción (puntos rojos).
- [ ] Integración con adaptadores de depuración ligeros (GDB/LLDB/Python).
- [ ] Panel de inspección de variables locales y pila de llamadas.

### 📌 Etapa 6: Optimización de Empaquetado y Distribución
- [ ] Perfil `release` ultra-optimizado (`lto = true`, `opt-level = 3`, `strip = true`).
- [ ] Instalador ligero de Windows o ejecutable portable `.zip` con todos sus recursos incluidos.

---

## ⌨️ Atajos de Teclado Principales

| Atajo | Acción |
| :--- | :--- |
| **`F5`** | Guardar y Ejecutar archivo actual (Salida integrada) |
| **`Shift + F5`** / **`Ctrl + C`** | Detener proceso en ejecución |
| **`Ctrl + ~`** / **`Ctrl + ñ`** / **`Ctrl + J`** | Mostrar / Ocultar panel dock inferior |
| **`Ctrl + S`** | Guardar archivo actual |
| **`Ctrl + Z`** / **`Ctrl + Y`** | Deshacer / Rehacer |
| **`Ctrl + F`** | Abrir menú de configuración y temas |
| **`Ctrl + A`** | Seleccionar todo el texto |
| **`Ctrl + C`** / **`Ctrl + V`** / **`Ctrl + X`** | Copiar / Pegar / Cortar |
