import type { components } from "./gen/api.ts";
export class ApiFailure extends Error {
  constructor(
    public code: string,
    message: string,
    public details?: components["schemas"]["ValuationErrorDetails"],
  ) {
    super(message);
  }
}
