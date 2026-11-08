# Flythrough, shots and movie capture

Shot files live in `src/bin/last_route/data/cinema/`. A shot has an id, a stage, a seed, a duration, actors
and a camera path (`t`, `x`, `y`, `z`, `yaw`, `pitch`). `trailer.json` lists the shot ids in order, with the seed.
The flythrough camera is a Catmull-Rom spline through the camera points. The cutscene runner plays the shots
on a fixed timeline, freezes input, pauses hunters on the host only, then returns control.

## Capture

    last_route --capture frames --capture-fps 30

Writes `frames/frame_000001.png`, ... at a fixed timestep of 1/FPS seconds. Assemble with ffmpeg:

    ffmpeg -framerate 30 -i frames/frame_%06d.png -c:v libx264 -pix_fmt yuv420p trailer.mp4
