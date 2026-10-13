// Stroke icons on a 16px grid, drawn with currentColor so they follow the button's color.
// Shared by Icon.svelte and the framework-free image viewer, which also runs on the share page.
export const PATHS = {
  plus: "M8 3.5v9M3.5 8h9",
  x: "M4 4l8 8M12 4l-8 8",
  "chevrons-right": "M4 4l4 4-4 4M9 4l4 4-4 4",
  message: "M2.5 3h11v8H9l-3 2.5V11H2.5zM5.5 7h5",
  copy: "M5.5 5.5h7v7h-7zM3.5 10.5v-7h7",
  check: "M3.5 8.5l3 3 6-7",
  external: "M9.5 3h3.5v3.5M13 3L7.5 8.5M11.5 9.5V13H3V4.5h3.5",
  "zoom-in": "M7 2.5a4.5 4.5 0 1 1 0 9a4.5 4.5 0 1 1 0-9zM10.5 10.5l3 3M5 7h4M7 5v4",
  "zoom-out": "M7 2.5a4.5 4.5 0 1 1 0 9a4.5 4.5 0 1 1 0-9zM10.5 10.5l3 3M5 7h4",
} as const;
export type IconName = keyof typeof PATHS;
