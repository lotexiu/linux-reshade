# Notes for upstream — depth buffer selection ideas (generic_depth add-on)

**Status: notes, not posted.** These are heuristics tested on a handful of games (Unity/DXVK, Unreal 3/DXVK, Unity native Vulkan, Roblox) on Linux with a layer that is not ReShade. They are offered as ideas to discuss, not as claims that ReShade's add-on is wrong.

What we changed relative to `examples/09-depth/generic_depth_addon.cpp`, and why:

1. **Sticky candidate (hysteresis).** The chosen depth buffer only changes after the challenger wins N consecutive frames (N=120). Roblox has several same-sized depth buffers with similar draw counts and the winner flipped frame to frame.
2. **A manual pick is remembered by image, not by list position.** The list order changes between frames and scenes; picking "candidate 2" used to alternate between different images. The pick is resolved to the image at request time, and inherited by the image with the same size/format if the game recreates it.
3. **Copy before a clear: pick the first *relevant* clear, not the largest.** In an Unreal 3 game under DXVK the number of clears per frame varies (16, 15, 39) so the index of the largest one is not stable, while "the first clear that erases at least 1/8 of the frame's largest erase and at least 32 draws" was always the same. The choice is adopted only after 8 consecutive frames agree and dropped after 30 frames if the frame no longer has that clear.
4. **If the rule-selected copy did not happen in a frame, keep the previous copy** instead of copying the live depth at effect time. (The live depth at that point can already contain what the rule is meant to avoid: a Unity game draws a large skewed quad into the depth buffer after the scene pass.)
5. **Copy at effect time with the layout of *now*.** The barrier is recorded each frame from the tracked layout instead of being baked when the effect chain is created; a baked barrier was wrong whenever the game left the image in another layout (loading screens, menus).
6. **`vkCmdClearAttachments` inside a dynamic-rendering pass.** The add-on comment says copying inside an active pass is invalid. We split the pass (`vkCmdEndRendering`, copy, `vkCmdBeginRendering` with `LOAD`) but only when it is valid: no query / conditional rendering / transform feedback opened inside the pass (they must end in the same render pass instance; DXVK uses them), and the depth/stencil `storeOp` is forced to `STORE` for the observed image so the first half keeps what the second half loads.
7. **Reversed depth detected automatically** from the pipelines' depth compare op (`GREATER` vs `LESS`) with the clear value as a fallback, instead of relying on the user setting a macro. Upside-down and logarithmic are still manual.
8. **A candidate that "does not fit" can still be chosen**, shown in yellow: different aspect ratio or no draws only mean the heuristic would not pick it. Only things that cannot be sampled at all (MSAA without SAMPLED usage) are blocked.

Caveats: every rule above was validated on few games; none of it replaces the add-on's behaviour where that behaviour works.
