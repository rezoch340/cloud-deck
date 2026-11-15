# Project instructions

- First read `HANDOFF.md` and `workstation-design/DESIGN.md` for the agreed product direction and current limitations.
- This is the formal Cloud Deck project. Source code belongs here. Builds, caches, downloads and temporary files belong under `/Volumes/CODEX_SCRATCH/`; verify its `.codex-scratch-volume` marker before creating scratch data. Existing durable user data may remain under `/Volumes/VoltDrive/AI/data/`.
- Work in Chinese with the user. Do not send phone notifications unless requested for the current task.
- The existing Swift/Python implementation is preserved as the migration baseline. The Windows/macOS Tauri client is in `client/`. Real macOS/Colab checks passed for OAuth, protected SSH/WSS, daemon deployment, forwarding, test-instance cleanup, built-in ComfyUI on T4 with Drive plus a 512×512 image execution, Drive server-side file verification, changed-file release blocking and application restart recovery. Full save-and-shutdown release and zero billing were also verified. All plugin behavior and Windows are not verified. Read `HANDOFF.md` for evidence and remaining limitations.
- The user chose HeroUI v3 + Tailwind CSS v4, not Ant Design. Reuse shared controls, application cards, editors and the workspace shell; do not hand-build inconsistent controls on each page.
- Run client installs/builds/tests through `node client/scripts/run.mjs <mode>` so dependencies, Cargo targets, generated assets and temporary files remain under the external work root. Never fall back to the internal disk when the external drive is missing.
- The target client manages connections, remote execution and forwarding internally, without requiring external SSH executables or terminal apps.
- ComfyUI is one application. Keep instance, app, connection and synchronization states independent. Disconnected or failed queries do not imply a stopped VM or zero billing.
- Show the allocated region from the actual Colab response/proxy, using the existing Swift/Python region mapping. Preserve the last allocated region after release; label cached/unknown values explicitly, never infer it from the GPU, client location or egress IP.
- Model and file lists must come from saved manifests and directory observations, with clear cached/error states. Preserve pinned ComfyUI/plugin versions; do not install DLSS.
- `auth/` contains local credentials and keys. Do not print them, commit them, upload them or attach them to reports.
- Do not allocate/release Colab instances or change Google authorization merely to verify local project setup. Obtain current task authorization for those actions.
- Do not start implementing the cross-platform rewrite as part of reading the handoff; follow the user's next instruction.
