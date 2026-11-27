import { fetchVNextWork } from '../api';
import type { OwnerWorkQueue } from '../api';
import { ownerQueueFromVNextWork } from './adapters';

export async function fetchVNextBackedOwnerQueue(initData: string): Promise<OwnerWorkQueue> {
  const vnextWork = await fetchVNextWork(initData);
  return ownerQueueFromVNextWork(vnextWork);
}
