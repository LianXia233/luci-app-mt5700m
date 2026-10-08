import React, { useEffect } from 'react';
import { Banner } from '@douyinfe/semi-ui';
import { startSharedStateFeed, useSharedStateFeed } from '@/services/stateCache';

/** App-wide status for the same StateCache/EventBus read by LuCI. */
const SharedStateStatus: React.FC = () => {
  const feed = useSharedStateFeed();

  useEffect(() => startSharedStateFeed(), []);

  if (feed.status === 'syncing' && Object.keys(feed.snapshot).length === 0) {
    return (
      <Banner
        type="info"
        closeIcon={null}
        title="正在同步设备共享状态"
        description="页面先显示缓存数据；状态由设备后台采集器异步更新。"
        style={{ marginBottom: 16 }}
      />
    );
  }

  if (feed.status === 'stale' || feed.status === 'error') {
    return (
      <Banner
        type="warning"
        closeIcon={null}
        title="设备状态暂不可用"
        description={feed.error || '正在等待状态源恢复；已有数据会保留。'}
        style={{ marginBottom: 16 }}
      />
    );
  }

  return null;
};

export default SharedStateStatus;
