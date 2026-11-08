// Pixel art as data. A rect is [x, y, w, h, color] in art units; y counts up from the
// floor line (so it's negative), and color is a palette key or a "#hex".
import { div } from "gpui-kit";

// px per art unit; the scene sets it from its height before drawing.
export let P = 3;
export const setScale = (p) => { P = p; };
export const BOX = 30; // units above the floor line a sprite's box reaches

export const mirror = (rects, width) => rects.map(([x, y, w, h, c]) => [width - x - w, y, w, h, c]);

export function rects(list, pal, p = P) {
  return list.map(([x, y, w, h, c]) =>
    div().absolute().left(x * p).top((BOX + y) * p).w(w * p).h(h * p).bg(pal[c] ?? c));
}

// One movable part of a sprite: its own box, so a loop on it moves all its rects.
export function layer(list, pal, width, p = P) {
  return div().absolute().left(0).top(0).w(width * p).h(BOX * p).children(rects(list, pal, p));
}

// People face right; the left-facing ones are mirrored in a box PERSON wide.
export const PERSON = 16;

const head = (x, y) => [
  [x, y, 6, 7, "skin"],
  [x + 6, y + 3, 1, 1, "skin"], // nose
  [x + 4, y + 2, 1, 2, "eye"],
  [x - 1, y - 1, 7, 3, "hair"],
  [x - 1, y + 2, 2, 3, "hair"],
];

export const STAND = {
  body: [...head(3, -19), [4, -12, 4, 1, "skin"], [3, -11, 6, 6, "shirt"], [3, -6, 6, 1, "belt"]],
  backLeg: [[4, -5, 2, 4, "pants2"], [4, -1, 3, 1, "shoe"]],
  frontLeg: [[6, -5, 2, 4, "pants"], [6, -1, 3, 1, "shoe"]],
  arm: [[5, -11, 2, 5, "sleeve"], [5, -6, 2, 1, "skin"]],
  cup: [[5, -11, 2, 3, "sleeve"], [6, -9, 3, 2, "sleeve"], [9, -12, 2, 3, "cup"], [8, -9, 1, 2, "skin"]],
  armUp: [[10, -17, 2, 6, "sleeve"], [10, -19, 2, 2, "skin"]],
  armUpBack: [[0, -17, 2, 6, "sleeve2"], [0, -19, 2, 2, "skin"]],
  // A box of desk things, carried out the door.
  box: [[5, -11, 2, 3, "sleeve"], [6, -9, 3, 2, "sleeve"], [10, -15, 1, 2, "leaf"], [11, -16, 1, 2, "leaf2"],
    [8, -13, 6, 5, "cardboard"], [8, -13, 6, 1, "cardboardTop"], [9, -9, 1, 1, "skin"]],
  pizza: [[5, -11, 2, 3, "sleeve"], [6, -9, 3, 2, "sleeve"], [8, -10, 8, 2, "pizzaBox"], [8, -12, 8, 2, "pizzaBox"], [9, -11, 4, 1, "pizzaLogo"]],
};

// Headphones and a mic, for agents working from another machine.
const headset = (x, y) => [[x, y - 2, 6, 1, "headset"], [x - 1, y + 2, 2, 3, "headset"], [x, y + 5, 3, 1, "headset"], [x + 3, y + 5, 1, 1, "headsetMic"]];
export const HEADSET = { stand: headset(3, -19), sit: headset(3, -23) };

// Sitting on a seat whose top is 8 units up, facing right.
export const SIT = {
  body: [...head(3, -23), [3, -16, 6, 6, "shirt"], [3, -10, 8, 2, "pants"], [10, -8, 2, 7, "pants"], [10, -1, 3, 1, "shoe"]],
  arm: [[5, -16, 2, 3, "sleeve"], [6, -14, 7, 2, "sleeve"], [13, -14, 2, 1, "skin"]],
  rest: [[5, -16, 2, 5, "sleeve"], [6, -11, 3, 1, "skin"]],
  lapArm: [[5, -16, 2, 3, "sleeve"], [6, -13, 4, 2, "sleeve"], [10, -13, 1, 1, "skin"]],
  laptop: [[7, -11, 6, 1, "laptop"], [12, -16, 1, 5, "laptop"], [11, -15, 1, 3, "laptopScreen"]],
  stool: [[2, -8, 8, 1, "stool"], [3, -7, 1, 7, "stoolLeg"], [8, -7, 1, 7, "stoolLeg"]],
};

export const SLOT = 48; // units per desk
export const DESK = {
  chair: [[0, -18, 2, 10, "chair"], [0, -8, 8, 2, "chair"], [3, -6, 2, 5, "chairLeg"], [1, -1, 6, 1, "chairLeg"]],
  desk: [
    [10, -12, 26, 2, "deskTop"], [10, -10, 26, 1, "deskEdge"],
    [13, -9, 2, 9, "deskLeg"], [33, -9, 2, 9, "deskLeg"],
    [24, -9, 8, 6, "deskSide"], [26, -7, 3, 1, "handle"],
    [12, -13, 5, 1, "kbd"],
    [21, -24, 12, 10, "mon"], [26, -14, 2, 1, "mon"], [24, -13, 6, 1, "mon"],
    [10, -14, 2, 2, "mug"], [9, -14, 1, 1, "mug"],
    [33, -13, 3, 1, "lamp"], [34, -19, 1, 6, "lamp"], [33, -22, 4, 3, "lampShade"],
  ],
  bell: [[18, -15, 3, 2, "bell"], [19, -16, 1, 1, "bell"], [18, -13, 3, 1, "bellBase"]],
  screen: [22, -23, 10, 8], // drawn by the scene: it changes with the agent
  bulb: [33, -19, 4, 1],
};

export const PIZZA_BOX = [
  [11, -18, 9, 6, "pizzaBox"], [13, -16, 5, 2, "pizzaLogo"], [11, -12, 9, 1, "pizzaBoxShade"],
  [12, -13, 7, 1, "pizza"], [13, -13, 1, 1, "pepperoni"], [16, -13, 1, 1, "pepperoni"], [18, -13, 1, 1, "pizzaCrust"],
];
export const MUG_PILE = [[10, -16, 2, 2, "mug2"], [10, -18, 2, 2, "cup"], [11, -20, 2, 2, "mug2"]];

export const PUMPKIN = {
  width: 8,
  rects: [[3, -7, 1, 1, "stem"], [1, -6, 6, 1, "pumpkin"], [0, -5, 8, 5, "pumpkin"], [2, -5, 1, 5, "pumpkinShade"], [5, -5, 1, 5, "pumpkinShade"],
    [1, -4, 2, 1, "pumpkinFace"], [5, -4, 2, 1, "pumpkinFace"], [2, -2, 4, 1, "pumpkinFace"]],
};
export const TREE = {
  width: 16,
  rects: [[7, -26, 2, 2, "star"], [6, -24, 4, 4, "tree"], [4, -20, 8, 5, "tree2"], [2, -15, 12, 6, "tree"], [1, -9, 14, 4, "tree2"],
    [7, -5, 2, 3, "trunk"], [5, -2, 6, 2, "pot"], [5, -18, 1, 1, "ornRed"], [9, -13, 1, 1, "ornBlue"], [4, -11, 1, 1, "ornGold"],
    [11, -8, 1, 1, "ornRed"], [7, -21, 1, 1, "ornGold"], [12, -12, 1, 1, "ornGold"], [3, -7, 1, 1, "ornBlue"]],
};

export const COUCH = {
  width: 34,
  rects: [
    [2, -15, 30, 7, "couchBack"], [0, -11, 4, 10, "couch"], [30, -11, 4, 10, "couch"],
    [3, -8, 28, 3, "couch"], [3, -5, 28, 4, "couchShade"], [16, -8, 1, 3, "couchShade"],
    [3, -1, 2, 1, "deskLeg"], [29, -1, 2, 1, "deskLeg"], [6, -13, 5, 4, "cushion"],
  ],
};

export const COOLER = {
  width: 10,
  rects: [
    [1, -14, 8, 14, "coolerBody"], [7, -14, 2, 14, "coolerShade"], [1, -1, 8, 1, "coolerShade"],
    [3, -10, 4, 2, "coolerShade"], [3, -11, 1, 1, "tapRed"], [6, -11, 1, 1, "tapBlue"],
    [2, -22, 6, 8, "water"], [3, -21, 1, 5, "waterHi"], [4, -23, 2, 1, "cap"],
    [9, -12, 1, 6, "cupStack"],
  ],
};

export const COUNTER = {
  width: 22,
  rects: [
    [0, -11, 22, 1, "counterTop"], [0, -10, 22, 10, "counter"], [10, -10, 1, 10, "counterLine"],
    [3, -7, 4, 1, "handle"], [14, -7, 4, 1, "handle"],
    [2, -19, 7, 8, "coffee"], [3, -18, 5, 2, "coffeeLight"], [4, -14, 3, 1, "coffeeShade"], [4, -13, 2, 2, "cup"],
    [12, -14, 3, 3, "mug2"], [16, -13, 2, 2, "cup"], [11, -12, 9, 1, "tray"],
  ],
};

export const PLANT = {
  width: 9,
  rects: [
    [4, -17, 2, 10, "leaf"], [1, -14, 2, 6, "leaf2"], [6, -15, 2, 7, "leaf2"],
    [0, -16, 2, 3, "leaf"], [7, -18, 2, 4, "leaf"], [3, -19, 2, 3, "leaf2"],
    [1, -7, 7, 1, "potRim"], [2, -6, 5, 6, "pot"],
  ],
};

// The office cat, facing right, in a box CAT units wide.
export const CAT = 14;
export const KITTY = {
  body: [[0, -8, 1, 4, "cat"], [1, -5, 1, 1, "cat"], [2, -5, 8, 3, "cat"], [4, -5, 1, 3, "catStripe"], [6, -5, 1, 3, "catStripe"],
    [3, -3, 6, 1, "catLight"], [9, -7, 4, 4, "cat"], [9, -8, 1, 1, "cat"], [12, -8, 1, 1, "cat"], [11, -6, 1, 1, "catEye"], [12, -4, 1, 1, "catLight"]],
  legsA: [[3, -2, 1, 2, "catStripe"], [8, -2, 1, 2, "cat"]],
  legsB: [[5, -2, 1, 2, "cat"], [9, -2, 1, 2, "catStripe"]],
  sit: [[1, -2, 4, 1, "cat"], [1, -3, 1, 1, "cat"], [4, -6, 5, 6, "cat"], [5, -4, 3, 3, "catLight"], [6, -10, 4, 4, "cat"],
    [6, -11, 1, 1, "cat"], [9, -11, 1, 1, "cat"], [8, -9, 1, 1, "catEye"], [5, -6, 1, 2, "catStripe"]],
  sleep: [[1, -2, 3, 1, "cat"], [2, -4, 9, 4, "cat"], [4, -4, 1, 3, "catStripe"], [6, -4, 1, 3, "catStripe"], [8, -5, 4, 3, "cat"],
    [8, -6, 1, 1, "cat"], [11, -6, 1, 1, "cat"], [9, -4, 2, 1, "catStripe"]],
};

export const PALETTE = {
  chair: "#3d4a6b", chairLeg: "#2a3149",
  deskTop: "#a0683f", deskEdge: "#7d4f2e", deskLeg: "#5c3a22", deskSide: "#8a5a35", handle: "#d9b37c",
  kbd: "#d9dde3", mon: "#2b2f3a", mug: "#c9b79c", lamp: "#3b3b44", lampShade: "#2f7a6a",
  bell: "#e0b43d", bellBase: "#6b5a2a",
  coolerBody: "#e9eef3", coolerShade: "#c3ccd6", tapRed: "#e05252", tapBlue: "#3d8be0",
  water: "#8fd0f5", waterHi: "#d6f0ff", cap: "#3d8be0", cupStack: "#ffffff",
  counter: "#7d6a58", counterTop: "#d8d2c8", counterLine: "#5e4f41",
  coffee: "#2d2d33", coffeeLight: "#58c26b", coffeeShade: "#18181c", mug2: "#e07a5f", tray: "#6b6b75",
  couch: "#5b6fb5", couchBack: "#4c5f9e", couchShade: "#3e4e85", cushion: "#e0b43d",
  leaf: "#3f9a55", leaf2: "#2f7a43", pot: "#c0643f", potRim: "#a3502f",
  eye: "#1d1d24", cup: "#ffffff", belt: "#3a2c22", shoe: "#2a2420",
  laptop: "#9aa3ad", laptopScreen: "#7fd1b9", stool: "#6b4a30", stoolLeg: "#3b2a1e",
  cardboard: "#c9995b", cardboardTop: "#a87c43", headset: "#2b2b33", headsetMic: "#e05252",
  pizzaBox: "#e2d3b5", pizzaBoxShade: "#bfae8c", pizzaLogo: "#c0392b", pizza: "#f2c14e", pepperoni: "#c0392b", pizzaCrust: "#d9a441",
  pumpkin: "#e8761e", pumpkinShade: "#c45f12", stem: "#5a7a2a", pumpkinFace: "#ffd76a",
  tree: "#2f7a43", tree2: "#3f9a55", trunk: "#6b4a30", star: "#ffd76a", ornRed: "#e05252", ornBlue: "#3d8be0", ornGold: "#e0b43d",
  cat: "#e39b4a", catLight: "#f5d3a3", catStripe: "#b86f2c", catEye: "#2a2a2a",
};

// Mugs by agent program: who drinks from what.
export const MUGS = { claude: "#d97757", codex: "#e8e8e8", gemini: "#4f86f7", cursor: "#2b2b2b", opencode: "#58b368", amp: "#9b6bdc" };

export const SKINS = ["#f6d2b0", "#e8b48a", "#c98d5f", "#9a6440", "#6e452b"];
export const HAIRS = ["#2a1c14", "#5a3520", "#c99640", "#1b1b1f", "#a8432d", "#d9d4c8", "#4b5bd6"];
export const SHIRTS = ["#e07a5f", "#3d8be0", "#58b368", "#9b6bdc", "#e0b43d", "#e05c8c", "#2fb3a8", "#f08c3a"];
export const PANTS = ["#2f3b55", "#3b3b44", "#4a3a2c", "#24425a"];

export const shade = (hex, f) => {
  const n = parseInt(hex.slice(1), 16);
  const c = [n >> 16, (n >> 8) & 255, n & 255].map((v) => Math.max(0, Math.min(255, Math.round(v * f))));
  return `#${c.map((v) => v.toString(16).padStart(2, "0")).join("")}`;
};

// A person's colors, the same every time for the same terminal.
export function look(address) {
  let h = 2166136261;
  for (const ch of address) h = Math.imul(h ^ ch.charCodeAt(0), 16777619) >>> 0;
  const pick = (list, salt) => list[(((h >>> salt) ^ (h >>> (salt + 7))) >>> 0) % list.length];
  const shirt = pick(SHIRTS, 0), pants = pick(PANTS, 5);
  return {
    ...PALETTE,
    skin: pick(SKINS, 3), hair: pick(HAIRS, 9), shirt, sleeve: shade(shirt, 0.88), sleeve2: shade(shirt, 0.72),
    pants, pants2: shade(pants, 0.75), salt: h,
  };
}

// The pizza delivery: red cap, red shirt.
export const COURIER = {
  ...PALETTE, skin: "#c98d5f", hair: "#c0392b", shirt: "#d94a3a", sleeve: "#c03f31", sleeve2: "#a33428",
  pants: "#2b2f3a", pants2: "#20232b", salt: 11,
};
