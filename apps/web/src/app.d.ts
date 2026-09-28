declare global {
  namespace App {
    interface Locals {
      requestId: string;
      requestSignal: AbortSignal;
    }
  }
}

export {};
