// Device class as reactive state: the phone gets the mobile shell (everything full screen), tablet and desktop — windows.
import { deviceClass, type DeviceClass } from './windows.svelte';
class Device {
  klass = $state<DeviceClass>(deviceClass());
  constructor() { if (typeof window !== 'undefined') window.addEventListener('resize', () => { this.klass = deviceClass(); }); }
  get phone() { return this.klass === 'phone'; }
}
export const device = new Device();
export const NAV_H = 56;
export const STATUS_H = 44;
