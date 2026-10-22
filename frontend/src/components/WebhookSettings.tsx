import React, { useEffect, useState } from 'react';
import { Eye, EyeOff } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Switch } from '@/components/ui/switch';

export interface WebhookSettingsData {
  enabled: boolean;
  url: string;
  header_name: string;
  header_value: string;
  mode: 'wake_word' | 'interval';
  interval_minutes: number;
  wake_word: string;
}

const DEFAULT_SETTINGS: WebhookSettingsData = {
  enabled: false,
  url: '',
  header_name: 'Authorization',
  header_value: '',
  mode: 'wake_word',
  interval_minutes: 5,
  wake_word: 'Emma',
};

const fieldClass =
  'w-full px-3 py-2 text-sm bg-white border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-1 focus:ring-blue-500 focus:border-blue-500';

export function WebhookSettings() {
  const [settings, setSettings] = useState<WebhookSettingsData>(DEFAULT_SETTINGS);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [showKey, setShowKey] = useState(false);
  const [testResult, setTestResult] = useState<string | null>(null);

  useEffect(() => {
    const load = async () => {
      try {
        const stored = await invoke<WebhookSettingsData>('get_webhook_settings');
        setSettings({
          ...DEFAULT_SETTINGS,
          ...stored,
          mode: stored.mode === 'interval' ? 'interval' : 'wake_word',
          interval_minutes: stored.interval_minutes > 0 ? stored.interval_minutes : 5,
          header_name: stored.header_name || 'Authorization',
          wake_word: stored.wake_word || 'Emma',
        });
      } catch (error) {
        console.error('Failed to load webhook settings:', error);
        toast.error('Could not load webhook settings', {
          description: error instanceof Error ? error.message : String(error),
        });
      } finally {
        setLoading(false);
      }
    };
    load();
  }, []);

  const update = (patch: Partial<WebhookSettingsData>) => {
    setSettings((current) => ({ ...current, ...patch }));
    setTestResult(null);
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      await invoke('set_webhook_settings', { settings });
      toast.success('Webhook settings saved');
    } catch (error) {
      console.error('Failed to save webhook settings:', error);
      toast.error('Failed to save webhook settings', {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setSaving(false);
    }
  };

  const handleTest = async () => {
    setTesting(true);
    setTestResult(null);
    try {
      const message = await invoke<string>('send_webhook_test', { settings });
      setTestResult(message);
      toast.success(message);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setTestResult(message);
      toast.error('Test webhook failed', { description: message });
    } finally {
      setTesting(false);
    }
  };

  if (loading) {
    return (
      <div className="animate-pulse mt-8">
        <div className="h-4 bg-gray-200 rounded w-1/4 mb-4"></div>
        <div className="h-8 bg-gray-200 rounded mb-4"></div>
      </div>
    );
  }

  return (
    <div className="space-y-6 mt-8 border-t pt-8">
      <div>
        <h3 className="text-lg font-semibold mb-2">Live meeting webhook</h3>
        <p className="text-sm text-gray-600">
          While a recording is active, Meetily can POST finalized transcript lines to a webhook.
          Paste the sender key exactly as the receiver shows it. Meetily does not add a Bearer prefix.
          The key is stored locally with your other API keys and is masked here.
        </p>
      </div>

      <div className="flex items-center justify-between p-4 border rounded-lg">
        <div className="flex-1">
          <div className="font-medium">Enable webhook</div>
          <div className="text-sm text-gray-600">
            Posts are sent only during an active recording. Failures are logged and do not stop transcription.
          </div>
        </div>
        <Switch
          checked={settings.enabled}
          onCheckedChange={(enabled) => update({ enabled })}
          disabled={saving}
        />
      </div>

      <div className="space-y-4 p-4 border rounded-lg">
        <div>
          <label className="block text-sm font-medium text-gray-700 mb-1" htmlFor="webhook-url">
            Webhook URL
          </label>
          <input
            id="webhook-url"
            type="url"
            className={fieldClass}
            placeholder="https://example.com/webhook"
            value={settings.url}
            autoComplete="off"
            spellCheck={false}
            onChange={(event) => update({ url: event.target.value })}
          />
        </div>

        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1" htmlFor="webhook-header-name">
              Auth header name
            </label>
            <input
              id="webhook-header-name"
              type="text"
              className={fieldClass}
              value={settings.header_name}
              autoComplete="off"
              spellCheck={false}
              onChange={(event) => update({ header_name: event.target.value })}
            />
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1" htmlFor="webhook-header-value">
              Sender key
            </label>
            <div className="relative">
              <input
                id="webhook-header-value"
                type={showKey ? 'text' : 'password'}
                className={`${fieldClass} pr-10`}
                value={settings.header_value}
                autoComplete="off"
                spellCheck={false}
                onChange={(event) => update({ header_value: event.target.value })}
              />
              <button
                type="button"
                className="absolute inset-y-0 right-0 px-3 text-gray-500 hover:text-gray-800"
                onClick={() => setShowKey((current) => !current)}
                aria-label={showKey ? 'Hide sender key' : 'Show sender key'}
              >
                {showKey ? <EyeOff className="w-4 h-4" /> : <Eye className="w-4 h-4" />}
              </button>
            </div>
          </div>
        </div>

        <div>
          <label className="block text-sm font-medium text-gray-700 mb-1" htmlFor="webhook-mode">
            Mode
          </label>
          <select
            id="webhook-mode"
            className={fieldClass}
            value={settings.mode}
            onChange={(event) => update({ mode: event.target.value as WebhookSettingsData['mode'] })}
          >
            <option value="wake_word">Wake word</option>
            <option value="interval">Interval</option>
          </select>
          <p className="text-xs text-gray-500 mt-2">
            {settings.mode === 'wake_word'
              ? 'When a finalized segment contains the wake word, send that segment plus about the last 2 minutes. One mention sends one post.'
              : 'Every N minutes, send transcript lines that are new since the last successful post. Nothing is sent when there is nothing new.'}
          </p>
        </div>

        {settings.mode === 'wake_word' ? (
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1" htmlFor="webhook-wake-word">
              Wake word
            </label>
            <input
              id="webhook-wake-word"
              type="text"
              className={fieldClass}
              value={settings.wake_word}
              onChange={(event) => update({ wake_word: event.target.value })}
            />
            <p className="text-xs text-gray-500 mt-2">Matched as a whole word, ignoring case.</p>
          </div>
        ) : (
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1" htmlFor="webhook-interval">
              Interval (minutes)
            </label>
            <input
              id="webhook-interval"
              type="number"
              min={1}
              max={1440}
              className={fieldClass}
              value={settings.interval_minutes}
              onChange={(event) => {
                const parsed = Number.parseInt(event.target.value, 10);
                update({ interval_minutes: Number.isFinite(parsed) ? parsed : 1 });
              }}
            />
          </div>
        )}

        <div className="flex flex-wrap items-center gap-3">
          <button
            type="button"
            onClick={handleSave}
            disabled={saving || testing}
            className="px-3 py-2 text-sm bg-blue-600 text-white rounded-md hover:bg-blue-700 disabled:opacity-50"
          >
            {saving ? 'Saving...' : 'Save'}
          </button>
          <button
            type="button"
            onClick={handleTest}
            disabled={saving || testing}
            className="px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-50 disabled:opacity-50"
          >
            {testing ? 'Sending...' : 'Send test'}
          </button>
          <span className="text-xs text-gray-500">
            Send test uses the values in this form, including unsaved edits. Save applies them to recordings.
          </span>
        </div>
        {testResult && (
          <p className="text-sm text-gray-700" role="status">
            {testResult}
          </p>
        )}
      </div>
    </div>
  );
}
