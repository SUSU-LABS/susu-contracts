import { xdr, scValToNative } from '@stellar/stellar-sdk';

export interface FeeUpdatedEvent {
  previous_fee_bps: number;
  new_fee_bps: number;
}

export interface TreasuryUpdatedEvent {
  previous_treasury: string;
  new_treasury: string;
}

export function decodeFeeUpdated(valueXdr: string): FeeUpdatedEvent {
  const value = xdr.ScVal.fromXDR(valueXdr, 'base64');
  const native = scValToNative(value);
  return {
    previous_fee_bps: native[0],
    new_fee_bps: native[1],
  };
}

export function decodeTreasuryUpdated(valueXdr: string): TreasuryUpdatedEvent {
  const value = xdr.ScVal.fromXDR(valueXdr, 'base64');
  const native = scValToNative(value);
  return {
    previous_treasury: native[0],
    new_treasury: native[1],
  };
}
