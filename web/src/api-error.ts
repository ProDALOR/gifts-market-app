export class ApiFailure extends Error {
  constructor(
    public code: string,
    message: string,
  ) {
    super(message);
  }
}
