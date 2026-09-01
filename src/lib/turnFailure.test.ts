import { describe, expect, it } from "vitest";

import { describeTurnFailure } from "./turnFailure";

describe("describeTurnFailure", () => {
  it("names the reason and points at the login flow when logging in would fix it", () => {
    const f = describeTurnFailure("authentication_failed", "api-refactor");
    expect(f.needsLogin).toBe(true);
    expect(f.message).toContain("api-refactor");
    expect(f.message).toContain("the login was refused");
    expect(f.message).toContain("Fix Claude login");
  });

  it("does not send account-level refusals to the login flow", () => {
    // Logging in again cannot lift a hold or settle a bill — pointing there
    // would waste the user's next click.
    for (const reason of ["account_on_hold", "billing_error"]) {
      const f = describeTurnFailure(reason);
      expect(f.needsLogin).toBe(false);
      expect(f.message).not.toContain("Fix Claude login");
    }
  });

  it("still says something useful for a rate limit", () => {
    const f = describeTurnFailure("rate_limit");
    expect(f.needsLogin).toBe(false);
    expect(f.message).toContain("the usage limit was reached");
  });

  it("falls back for an unknown or missing reason instead of going blank", () => {
    // A reason enum a future CLI adds must not produce an empty message.
    expect(describeTurnFailure("some_future_kind").message).toContain("the API refused it");
    expect(describeTurnFailure(undefined).message).toContain("the API refused it");
    expect(describeTurnFailure(null).message).toContain("The turn stopped");
  });

  it("omits the session name when the event could not be bound to one", () => {
    expect(describeTurnFailure("server_error").message.startsWith("The turn stopped")).toBe(true);
  });
});
