// Typed API client for the LiveServer string store.
//
// Same pattern as api.ts — uses Hono RPC (hc) with the server's exported
// route type for end-to-end type safety.

import { hc } from "hono/client";
import type { StringsApiType } from "../../server/strings";

/// Typed Hono RPC client for /strings endpoints.
export const stringsApi = hc<StringsApiType>("/strings");
