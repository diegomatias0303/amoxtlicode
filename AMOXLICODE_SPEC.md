# Specification Document: AmoxliCode IDE (Ultra-Light Multimedia IDE)

## 1. Project Overview & Architectural Vision
The goal is to build an ultra-lightweight, high-performance integrated development environment ("AmoxliCode") that achieves the execution speed and low memory footprint of Notepad++ (~15-30MB RAM) while integrating modern hardware-accelerated rendering and robust language intelligence. The name derives from *Amoxtli* (the Nahuatl term for the codices used to record knowledge) and *Code*.

Unlike traditional Electron/Chromium editors (VS Code), this system treats the text editor as a high-performance **2D Graphical Engine**. It utilizes a layered GPU architecture where video decoding (`.mp4`/`.gif`) runs in the background layer, and modern text layout engines render code characters directly on top via a unified render loop executing at a locked 60 FPS (V-Sync).

### Technology Stack
* **Language:** Rust (Stable)
* **Windowing & Event Handling:** `winit`
* **Graphics API Layer:** `wgpu` (Targeting Vulkan / DirectX 12 backend pipelines)
* **Text Layout & Font Rendering:** `cosmic-text` (Leveraging hardware glyph caching)
* **Multimedia Decoding Engine:** `ffmpeg-next` (Safe Rust bindings for FFmpeg)
* **Syntax Parsing Engine:** `tree-sitter` (Incremental multi-language structural analysis)
* **Intelligence Protocol:** `lsp-types` (Language Server Protocol)

---

## 2. System Pipeline Architecture
The execution cycle avoids traditional GDI/Win32 drawing to prevent screen flickering. It implements a strict **Double-Buffered Render Loop**:

```
 [User Inputs] ──> [Update State Layer] ──> [Hardware Render Layer] ──> [Present]
 (Key/Mouse)        (Decode next video frame) (Layer 0: BG + Layer 1: Text)   (GPU FrontBuffer)
```

### Layer Layering Specifications:
1.  **Layer 0 (Background):** A dynamic hardware texture continuously overwritten by decoded RGBA matrix frames parsed by FFmpeg from an isolated background worker thread. Blending operations enforce a hardcoded opacity threshold (Default: `0.15`).
2.  **Layer 1 (Foreground):** Alpha-blended text glyph cache layout. Glyphs map directly above Layer 0 inside the same command encoder submission pass, colored dynamically by parser tokens.

---

## 3. Step-by-Step Implementation Blueprints (Sequential Prompts)

You must process this build linearly. Do not implement a stage until the prior compilation steps pass verification.

### Phase 1: Engine Initialization & Windowing (The Shell)
**Context & Goal:** Establish the hardware-accelerated viewport using `winit` and `wgpu`.
```text
PROMPT TO EXECUTE:
Act as a Principal Systems Engineer expert in Rust, winit, and wgpu. Provide a complete boilerplate for a windowed application named 'AmoxliCode'. 
1. Generate a robust Cargo.toml specifying dependencies for 'winit', 'wgpu', and 'env_logger'.
2. In main.rs, setup a structured event loop (`winit`) managing a window.
3. Initialize the `wgpu` Instance, Adapter, Device, and Queue targeting high-performance hardware configurations.
4. Implement a clear render loop executing an structural clearing pass (Color Pass) to a specific dark background color.
Ensure strict error handling with `Result` types and avoid unwrap expressions within the core event processing loop.
```

### Phase 2: Background Layer Integration (The Video Engine)
**Context & Goal:** Spawning an asynchronous processing worker that streams video frames into textures without causing main loop blocking.
```text
PROMPT TO EXECUTE:
We have a working wgpu state rendering a solid background. Now implement the Background Layer (Layer 0) for AmoxliCode.
1. Add `ffmpeg-next` and `crossbeam-channel` dependencies to Cargo.toml.
2. Design a module `video.rs` containing a `VideoDecoder` struct. This structural unit must load a local MP4 video/GIF asset, read packet frames sequentially, and transcode raw video packets into a continuous stream of raw RGBA byte vectors.
3. Run this decoding process inside an isolated background thread, passing frames down a channel to prevent main thread blocking.
4. In the main wgpu render loop, receive frames from the channel, update a pre-allocated `wgpu::Texture` using `queue.write_texture`, and render this texture as a screen-aligned quad layout.
5. Apply a pipeline blending factor (Fragment Shader Stage) to enforce a static 15% opacity layer so text remains readable.
```

### Phase 3: Glyphs and Layout Systems (The Canvas)
**Context & Goal:** Introducing high-performance text handling over existing dynamic textures.
```text
PROMPT TO EXECUTE:
The video layer renders correctly at 60 FPS. We must now incorporate the Text Engine (Layer 1) using `cosmic-text`.
1. Append `cosmic-text` to Cargo.toml dependencies.
2. Create `editor.rs` to initialize a `FontSystem`, `SwashCache`, and an explicit text `Buffer`. Set the font selection layout to look specifically for Monospace types (e.g., Fira Code, Consolas).
3. Build a pipeline processing pass that takes the text buffer string data, converts characters into graphical glyph primitives via the swash cache, and generates a dynamic texture atlas overlay.
4. Merge this pass into the existing main command encoder pass. Draw these text glyph textures directly on top of the Layer 0 background texture utilizing standard alpha transparency channels.
```

### Phase 4: Full State Interaction & Mutation (The Editor Core)
**Context & Goal:** Transforming a visual layout system into a responsive text IDE input structure.
```text
PROMPT TO EXECUTE:
Stage 4: Bind interactive state changes. Map system inputs from `winit` directly into the editor state machine.
1. Intercept `WindowEvent::ReceivedCharacter` and `WindowEvent::KeyboardInput` from the event pump.
2. Wire character inputs directly into the `cosmic-text` Buffer layout stack to allow real-time writing.
3. Explicitly program operations handling standard typing behavior:
   - Carriage Returns ('Enter' key) must call terminal buffer line breaks.
   - Backspace keys must correctly trigger safe pop operations out of string storage.
   - Arrow keys must move a structural index representation representing a visual blinking cursor block.
4. Ensure text recalculations happen dynamically inside the render loop without blocking or lagging the FFmpeg texture updates.
```

### Phase 5: Multi-Language Syntax Highlighting (Tree-Sitter Integration)
**Context & Goal:** Adding syntax coloring for major languages: C, Java, JavaScript, HTML, CSS, and SQL.
```text
PROMPT TO EXECUTE:
Now we need to add structural Syntax Highlighting to Layer 1 of AmoxliCode to handle C, Java, SQL, JS, HTML, and CSS.
1. Add `tree-sitter` and language parsers (`tree-sitter-c`, `tree-sitter-java`, `tree-sitter-javascript`, `tree-sitter-html`, `tree-sitter-css`, `tree-sitter-sql`) to Cargo.toml.
2. In editor.rs, parse the text buffer using the appropriate Tree-Sitter grammar based on the current file extension.
3. Map the generated Abstract Syntax Tree (AST) tokens to specific colors (e.g., Keywords = Purple, Strings = Green, Functions = Blue).
4. Update the cosmic-text layout loop to apply these dynamic color attributes to the text glyphs before rendering them over the background video.
```

### Phase 6: Floating IntelliSense Engine (LSP Autocompletion Popup)
**Context & Goal:** Displaying an autocomplete popup near the cursor based on language intelligence inputs.
```text
PROMPT TO EXECUTE:
Implement the IntelliSense/Autocompletion Popup engine for AmoxliCode.
1. Add `lsp-types` to manage standard language server communication protocol events.
2. Create a UI state `CompletionPopup` that triggers when specific trigger characters (like '.', '->', or after typing 2 alphanumeric characters) are processed.
3. Render a small overlay bounding-box (floating menu) directly at the current visual cursor pixel coordinates using wgpu.
4. Populate this menu with active LSP suggestions or language keywords. 
5. Intercept 'Tab' or 'Arrow Down/Up' key events to navigate and inject the selected autocompletion token directly into the active string buffer position.
```

---

## 4. Optimization Guidelines (Enforce via Compiler)
To guarantee your solution fits the strict performance goals of this architecture, verify the codebase strictly conforms to the following constraints:
* **Asynchronous Frame Loading:** The main rendering thread must NEVER perform I/O read operations or wait for heavy decoding loops.
* **Zero-Copy Memory Layouts:** Map decoded vector pixels directly across pre-allocated buffers. Avoid allocation loops inside the render iteration block.
* **V-Sync Enforcement:** Set your Presentation Mode configurations within the WGPU Swapchain descriptor to explicitly target `PresentMode::Fifo` to preserve exact monitor refresh syncing.
