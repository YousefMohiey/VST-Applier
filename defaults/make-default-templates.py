import json
import os

WORK = r"C:\Users\Administrator\Desktop\_defaults_work"
REPO = r"C:\Users\Administrator\projects\vst-applier"

src = json.load(open(os.path.join(WORK, "VstApplier", "session.json"), encoding="utf-8-sig"))

path_map = {
    "reaeq-standalone.dll": "{APP}\\Plugins\\ReaPlugs\\reaeq-standalone.dll",
    "reagate-standalone.dll": "{APP}\\Plugins\\ReaPlugs\\reagate-standalone.dll",
    "rnnoise_stereo.dll": "{APP}\\Plugins\\rnnoise\\vst\\rnnoise_stereo.dll",
}

plugins = []
for entry in src["Plugins"]:
    file_name = entry["Path"].split("\\")[-1].lower()
    if file_name not in path_map:
        raise SystemExit(f"unmapped plugin: {file_name}")
    plugins.append(
        {
            "Name": entry["Name"],
            "Path": path_map[file_name],
            "Format": entry["Format"],
            "Enabled": entry["Enabled"],
            "State": entry["State"],
        }
    )

profile = {
    "SchemaVersion": 1,
    "Name": "Main",
    "ActiveProfile": "Main",
    "PluginFolder": "{APP}\\Plugins",
    "InputDeviceId": None,
    "InputDeviceName": None,
    "OutputDeviceId": None,
    "OutputDeviceName": None,
    "BufferSize": 512,
    "Plugins": plugins,
}

out_dir = os.path.join(REPO, "defaults", "user")
os.makedirs(os.path.join(out_dir, "profiles"), exist_ok=True)

for relative, data in [("session.json", profile), ("profiles/Main.json", profile)]:
    with open(os.path.join(out_dir, relative), "w", encoding="utf-8") as handle:
        json.dump(data, handle, indent=2, ensure_ascii=False)

with open(os.path.join(out_dir, "settings.json"), "w", encoding="utf-8") as handle:
    json.dump(
        {
            "AutoStartMicrophone": True,
            "CloseToTray": True,
            "StartWithWindows": True,
            "KeepCableClean": True,
        },
        handle,
        indent=2,
    )

print("--- Main.json ---")
print(open(os.path.join(out_dir, "profiles", "Main.json"), encoding="utf-8").read()[:700])
print("--- settings.json ---")
print(open(os.path.join(out_dir, "settings.json"), encoding="utf-8").read())
