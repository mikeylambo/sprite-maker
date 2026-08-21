import { describe, expect, test } from "bun:test";
import { GENERATION_PRESETS, normalizeGenerationProfile, profileFromGame } from "../src/lib/generation-profiles";

describe("generation profile defaults", () => {
  test("uses a 128 by 128 mid-quality canvas by default", () => {
    const profile = normalizeGenerationProfile(null);
    expect(profile.quality).toBe("mid");
    expect([profile.width, profile.height]).toEqual([128, 128]);
    expect(profile.profileVersion).toBe(8);
  });

  test("upgrades saved preset profiles while preserving custom dimensions", () => {
    const legacy = normalizeGenerationProfile({ profileVersion: 7, quality: "mid", width: 64, height: 64 });
    expect([legacy.width, legacy.height]).toEqual([128, 128]);

    const custom = normalizeGenerationProfile({ profileVersion: 7, quality: "custom", width: 96, height: 80 });
    expect([custom.width, custom.height]).toEqual([96, 80]);
    expect(GENERATION_PRESETS.mid.width).toBe(128);
  });
});

describe("game profile seeding", () => {
  test("seeds a new chat with the game's canvas and playback", () => {
    const profile = profileFromGame({ baseUnitPx: 64, fps: { default: 10 } });
    expect([profile.width, profile.height]).toEqual([64, 64]);
    expect(profile.fps).toBe(10);
    expect(profile.quality).toBe("custom");
  });

  test("falls back to the mid preset without a game profile", () => {
    const profile = profileFromGame(undefined);
    expect(profile.quality).toBe("mid");
    expect([profile.width, profile.height]).toEqual([128, 128]);
  });

  test("ignores out-of-range or missing game values", () => {
    const oversized = profileFromGame({ baseUnitPx: 4096, fps: { default: 999 } });
    expect(oversized.quality).toBe("mid");

    const partial = profileFromGame({ baseUnitPx: 32 });
    expect([partial.width, partial.height]).toEqual([32, 32]);
    expect(partial.fps).toBe(GENERATION_PRESETS.mid.fps);
  });
});
