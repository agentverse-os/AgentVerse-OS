// Класс устройства как реактивное состояние: телефон получает мобильную оболочку (всё на весь экран), планшет и десктоп — окна.
import { deviceClass, type DeviceClass } from './windows.svelte';
class Device {
  klass = $state<DeviceClass>(deviceClass());
  constructor() { if (typeof window !== 'undefined') window.addEventListener('resize', () => { this.klass = deviceClass(); }); }
  get phone() { return this.klass === 'phone'; }
}
export const device = new Device();
export const NAV_H = 56;
export const STATUS_H = 44;
