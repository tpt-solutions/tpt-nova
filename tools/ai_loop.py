#!/usr/bin/env python3
"""TPT Nova — external AI injection loop (proof of the telemetry <-> control round-trip).

This script demonstrates the closed-loop self-debugging described in the spec:

  1. Read `nova-telemetry.json` (emitted by nova-app on an interval).
  2. Find the cube entity's current rotation (Euler XYZ) from its Transform.
  3. Nudge the yaw by a small delta (the "AI decision").
  4. Write `nova-control.json` so the running engine hot-applies the new rotation
     without a restart.

Run nova-app in one terminal, then run this script in another:

    python3 tools/ai_loop.py

It will keep spinning the cube live until interrupted.
"""

import json
import math
import os
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TELEMETRY = os.path.join(ROOT, "nova-telemetry.json")
CONTROL = os.path.join(ROOT, "nova-control.json")

# Euler XYZ order must match glam::EulerRot::XYZ used in nova-app.
DELTA_YAW = 0.15  # radians per cycle
SLEEP = 0.5       # seconds between cycles


def quat_to_euler_xyz(q):
    x, y, z, w = q
    # Roll (x), Pitch (y), Yaw (z) from a quaternion (glam EulerRot::XYZ).
    sinr_cosp = 2.0 * (w * x + y * z)
    cosr_cosp = 1.0 - 2.0 * (x * x + y * y)
    roll = math.atan2(sinr_cosp, cosr_cosp)

    sinp = 2.0 * (w * y - z * x)
    pitch = math.asin(max(-1.0, min(1.0, sinp)))

    siny_cosp = 2.0 * (w * z + x * y)
    cosy_cosp = 1.0 - 2.0 * (y * y + z * z)
    yaw = math.atan2(siny_cosp, cosy_cosp)
    return roll, pitch, yaw


def find_cube(telemetry):
    for ent in telemetry.get("entities", []):
        comps = ent.get("components", {})
        if "Mesh" in comps:
            t = comps.get("Transform")
            if t and "rotation" in t:
                return t["rotation"]
    return None


def main():
    print(f"[ai_loop] watching {TELEMETRY}")
    print(f"[ai_loop] writing  {CONTROL}")
    while True:
        if not os.path.exists(TELEMETRY):
            time.sleep(SLEEP)
            continue
        try:
            with open(TELEMETRY) as f:
                telemetry = json.load(f)
        except (json.JSONDecodeError, OSError):
            time.sleep(SLEEP)
            continue

        quat = find_cube(telemetry)
        if quat is None:
            time.sleep(SLEEP)
            continue

        roll, pitch, yaw = quat_to_euler_xyz(quat)
        new_yaw = yaw + DELTA_YAW
        print(
            f"[ai_loop] tick={telemetry.get('tick')} "
            f"read yaw={yaw:+.3f} -> write yaw={new_yaw:+.3f}"
        )

        control = {"set_rotation": {"x": roll, "y": new_yaw, "z": pitch}}
        with open(CONTROL, "w") as f:
            json.dump(control, f, indent=2)

        time.sleep(SLEEP)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("\n[ai_loop] stopped")
