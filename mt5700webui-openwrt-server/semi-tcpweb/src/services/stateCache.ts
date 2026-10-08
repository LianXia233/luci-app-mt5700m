import { useCallback, useSyncExternalStore } from 'react';
import { ATService, type ATResponse, type StateEntry, type StateSnapshot } from '@/services/at';

/**
 * Shared modem state feed for the built-in WebUI.
 *
 * This is deliberately the same Rust daemon StateCache / EventBus consumed by
 * LuCI (`mt5700.cached` / `cachedSnapshot`), not a second browser-side polling
 * or AT acquisition path. A snapshot is a zero-AT SWR read; subsequent values
 * arrive as EventBus pushes. The periodic snapshot is also zero-AT and recovers
 * missed events; unchanged topic entries retain identity to avoid re-rendering.
 */
export type StateFeedStatus = 'idle' | 'syncing' | 'ready' | 'stale' | 'error';

export interface SharedStateFeed {
  snapshot: StateSnapshot;
  status: StateFeedStatus;
  error: string;
  updatedAt: number;
}

const EMPTY_FEED: SharedStateFeed = {
  snapshot: {},
  status: 'idle',
  error: '',
  updatedAt: 0,
};

const SNAPSHOT_INTERVAL_MS = 15_000;
const EVENT_TOPIC: Record<string, string> = {
  signal: 'signal',
  'signal.updated': 'signal',
  'network.updated': 'network',
  'cell.updated': 'cell',
  'temperature.updated': 'temperature',
  'traffic.updated': 'traffic',
  'netrate.updated': 'netrate',
  'registration.updated': 'registration',
  'endc.updated': 'endc',
  'txpower.updated': 'txpower',
  'nr_txpower.updated': 'nr_txpower',
  'sim.updated': 'sim',
  'modem.info': 'modem',
  'ca.updated': 'ca',
  'qos.updated': 'qos',
};

let feed: SharedStateFeed = EMPTY_FEED;
let eventRevision = 0;
let topicRevision: Record<string, number> = {};
let pendingSnapshot: Promise<StateSnapshot | null> | null = null;
let feedUsers = 0;
let stopFeed: (() => void) | null = null;
const listeners = new Set<() => void>();

const emit = () => listeners.forEach((listener) => listener());

const setFeed = (next: SharedStateFeed) => {
  feed = next;
  emit();
};

const subscribeFeed = (listener: () => void) => {
  listeners.add(listener);
  return () => listeners.delete(listener);
};

export const getSharedStateFeed = (): SharedStateFeed => feed;

export function useSharedStateFeed(): SharedStateFeed {
  return useSyncExternalStore(subscribeFeed, getSharedStateFeed, getSharedStateFeed);
}

/** Subscribe to one topic; unrelated EventBus updates do not re-render the page. */
export function useSharedStateTopic(topic: string): StateSnapshot[string] | undefined {
  const getTopic = useCallback(() => feed.snapshot[topic], [topic]);
  return useSyncExternalStore(subscribeFeed, getTopic, getTopic);
}

const sameTopicValue = (left: StateEntry | undefined, right: StateEntry | undefined): boolean =>
  !!left &&
  !!right &&
  left.fresh === right.fresh &&
  JSON.stringify(left.value) === JSON.stringify(right.value);

const stabilizeSnapshot = (incoming: StateSnapshot): StateSnapshot => {
  const stable: StateSnapshot = {};
  Object.entries(incoming).forEach(([topic, entry]) => {
    const previous = feed.snapshot[topic];
    stable[topic] = sameTopicValue(previous, entry) ? previous : entry;
  });
  return stable;
};

const topicFromEvent = (response: ATResponse): string | null => {
  if (
    !('type' in response) ||
    typeof response.type !== 'string' ||
    !response.data ||
    typeof response.data !== 'object'
  ) {
    return null;
  }
  return EVENT_TOPIC[response.type] || null;
};

const applyEvent = (response: ATResponse) => {
  const topic = topicFromEvent(response);
  if (!topic) return;

  eventRevision += 1;
  topicRevision[topic] = eventRevision;
  const value = response.data as Record<string, unknown>;
  const entry: StateEntry = { value, fresh: true, age_ms: 0, source: 'event' };
  const previous = feed.snapshot[topic];
  if (sameTopicValue(previous, entry) && feed.status === 'ready' && !feed.error) return;

  const snapshot = {
    ...feed.snapshot,
    [topic]: sameTopicValue(previous, entry) ? previous : entry,
  };
  setFeed({ snapshot, status: 'ready', error: '', updatedAt: Date.now() });
};

/** Fetch the current StateCache snapshot; requests are coalesced application-wide. */
export function refreshSharedStateFeed(): Promise<StateSnapshot | null> {
  if (pendingSnapshot) return pendingSnapshot;

  const at = ATService.getInstance();
  if (!at.isReady()) return Promise.resolve(null);

  const revisionsAtStart = { ...topicRevision };
  if (!Object.keys(feed.snapshot).length) {
    setFeed({ ...feed, status: 'syncing', error: '' });
  }

  const request = at
    .requestSnapshot()
    .then((snapshot) => {
      if (!snapshot) {
        const hasCachedValues = Object.keys(feed.snapshot).length > 0;
        setFeed({
          ...feed,
          status: hasCachedValues ? 'stale' : 'error',
          error: '无法读取设备共享状态，保留已有数据并自动重试。',
        });
        return null;
      }

      // Keep any topic event that arrived while the snapshot was in flight;
      // it is newer than the snapshot request and must win over it.
      const merged: StateSnapshot = { ...feed.snapshot, ...snapshot };
      Object.keys(topicRevision).forEach((topic) => {
        if (
          (topicRevision[topic] || 0) > (revisionsAtStart[topic] || 0) &&
          feed.snapshot[topic]
        ) {
          merged[topic] = feed.snapshot[topic];
        }
      });

      const stable = stabilizeSnapshot(merged);
      const unchanged =
        Object.keys(stable).length === Object.keys(feed.snapshot).length &&
        Object.keys(stable).every((topic) => stable[topic] === feed.snapshot[topic]);
      if (unchanged && feed.status === 'ready' && !feed.error) return stable;

      setFeed({ snapshot: stable, status: 'ready', error: '', updatedAt: Date.now() });
      return stable;
    })
    .catch(() => {
      const hasCachedValues = Object.keys(feed.snapshot).length > 0;
      setFeed({
        ...feed,
        status: hasCachedValues ? 'stale' : 'error',
        error: '设备共享状态暂不可用，保留已有数据并自动重试。',
      });
      return null;
    })
    .finally(() => {
      if (pendingSnapshot === request) pendingSnapshot = null;
    });

  pendingSnapshot = request;
  return request;
}

/** Start one app-wide EventBus subscription and zero-AT snapshot recovery loop. */
export function startSharedStateFeed(): () => void {
  feedUsers += 1;
  if (feedUsers > 1) {
    let released = false;
    return () => {
      if (released) return;
      released = true;
      feedUsers = Math.max(0, feedUsers - 1);
    };
  }

  const at = ATService.getInstance();
  let active = true;
  const onEvent = (response: ATResponse) => {
    if (active) applyEvent(response);
  };
  const refresh = () => {
    if (active && at.isReady()) void refreshSharedStateFeed();
  };

  at.subscribe(onEvent);
  const offConnect = at.onConnectSuccess(refresh);
  const offState = at.onConnectionStateChange(({ state, error }) => {
    if (state === 'connected') {
      if (!Object.keys(feed.snapshot).length) {
        setFeed({ ...feed, status: 'syncing', error: '' });
      }
      refresh();
      return;
    }

    if (state === 'connecting' || state === 'authenticating') {
      if (!Object.keys(feed.snapshot).length) setFeed({ ...feed, status: 'syncing', error: '' });
      return;
    }

    if (state === 'reconnecting' || state === 'disconnected' || state === 'error') {
      const hasCachedValues = Object.keys(feed.snapshot).length > 0;
      setFeed({
        ...feed,
        status: hasCachedValues ? 'stale' : state === 'error' ? 'error' : 'idle',
        error: error || (hasCachedValues ? '设备连接中断，保留最近数据并等待恢复。' : ''),
      });
    }
  });

  if (at.isReady()) refresh();
  const timer = window.setInterval(refresh, SNAPSHOT_INTERVAL_MS);

  stopFeed = () => {
    active = false;
    window.clearInterval(timer);
    at.unsubscribe(onEvent);
    offConnect();
    offState();
  };

  let released = false;
  return () => {
    if (released) return;
    released = true;
    feedUsers = Math.max(0, feedUsers - 1);
    if (feedUsers === 0) {
      stopFeed?.();
      stopFeed = null;
    }
  };
}
