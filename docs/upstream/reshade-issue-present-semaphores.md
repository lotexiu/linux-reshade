# Draft issue for crosire/reshade — present wait semaphores vs. VK_EXT_swapchain_maintenance1 / present_id2

**Status: draft, not posted.** Not verified with ReShade itself (see "What I did not verify").

## Title
Vulkan layer replaces the present's wait semaphores: this broke our own layer with applications that use `VK_EXT_swapchain_maintenance1` present fences and `VK_KHR_present_id2`

## Summary
I maintain a small ReShade-compatible Vulkan layer for Linux (it runs ReShade FX effects through a layer instead of through Wine). In `vkQueuePresentKHR` it did what ReShade's Vulkan hook does: it submits the effect work waiting on the present's wait semaphores and then **replaces the present's wait list with the semaphore the effect submit signals**.

That worked with every application I tried until I ran a game through DXVK 3.1.1-97 (the one in Proton Experimental), which creates its swapchain with `VK_KHR_present_id2` / `present_wait2` flags and attaches `VkSwapchainPresentFenceInfoEXT` and `VkSwapchainPresentModeInfoEXT` to every present. With heavy effect chains the GPU hung within a minute or two (NVIDIA RTX 3060, driver 615.78.08: Xid 69 "Class Error" and once Xid 109 "CTX SWITCH TIMEOUT"). The same game and the same effects with GE-Proton 11-7 (DXVK 3.1-19, which does not use those extensions) never hung.

## What fixed it
Not replacing the wait list. The effect submit waits on the application's semaphores **and signals the same semaphores again**; the present then waits on exactly the semaphores the application handed over, and the rest of the present info (including the `pNext` chain with the present fence / present id) is untouched. Validation layers stay clean. With that change the same game, same Proton Experimental, same heavy preset ran without any Xid (it hung every time before).

## Why I think it may matter for ReShade
`source/vulkan/vulkan_hooks_swapchain.cpp` (`vkQueuePresentKHR`) ends with

```cpp
// Override wait semaphores based on the last queue submit
present_info.waitSemaphoreCount = submit_info.waitSemaphoreCount;
present_info.pWaitSemaphores = submit_info.pWaitSemaphores;
```

and `command_list_immediate_impl::flush` points `wait_semaphore_info` at the semaphore it signaled. So the present stops waiting on the semaphores the application passed. For an application that uses `VkSwapchainPresentFenceInfoEXT` ("signaled when the semaphores waited on by this present can be reused"), or that recycles its render-finished semaphores from `present_id` / present-wait signals, the semaphores it handed over are no longer the ones the presentation engine waits on. I do not know which of the two (the fence/id bookkeeping, or the changed ordering) triggers the hang on NVIDIA; I only know that not replacing them removes it.

There is already a branch in the same function for RTX Remix that signals the application's semaphores again on the graphics queue ("Signal them on the present queue again..."); the approach above is the same idea applied to the effect submit.

A sketch of the alternative (not tested inside ReShade):

```cpp
// in flush_immediate_command_list(wait_semaphore_info):
if (wait_semaphore_info->waitSemaphoreCount != 0) {
    submit_info.signalSemaphoreCount = wait_semaphore_info->waitSemaphoreCount;
    submit_info.pSignalSemaphores    = wait_semaphore_info->pWaitSemaphores;   // re-signal what we waited on
    // ...and do not override present_info.pWaitSemaphores afterwards
}
```

A binary semaphore may be waited on and signaled again by the same batch, so this is valid Vulkan.

## What I did not verify
- I did not run ReShade's own Vulkan layer. ReShade under Wine/Proton hooks D3D11/DXGI, so it never reaches this code path; this only applies to native Vulkan applications that use the new present extensions.
- I have no Windows/Linux-native Vulkan test case that uses `VK_EXT_swapchain_maintenance1` with ReShade.
- The hang could also depend on the effect chain being heavy; the draft would be stronger with a reproduction on ReShade.

## Other things that turned out to matter in the same area
- A per-`VkQueue` mutex around the application's `vkQueueSubmit*` and our own submit (ReShade already has `queue_impl->_mutex`).
- Logging the device extensions and the `pNext` chains of the first presents made the difference visible; a one-line log of the swapchain create flags and the present `pNext` types would help anyone debugging this.

Happy to open a PR if you think the change is worth having; I can only test it on Linux with a native Vulkan application.
