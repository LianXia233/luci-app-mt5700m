// 模组的主动上报（URC）里，前端只剩一件事：判断一段裸文本是不是上报，
// 避免把它当成命令应答串了等待中的命令（`isUnsolicitedText`）。
//
// 上报的**解析**全在后端：`transport/urc.rs` 的 Dispatcher 把 ^HCSQ / ^RSSI /
// ^PDCPDATAINFO / +CUSD / ^REJINFO … 解成结构化事件推给前端
// （`signal.updated`、`pdcp_data`、`sms.ussd`、`network.reject`，见
// services/events.ts），所以这里不再保留第二套解析表。PDCPData 只是
// `pdcp_data` 事件的负载类型。

export interface PDCPData {
  id: number;
  pduSessionId: number;
  discardTimerLen: number;
  avgDelay: number;
  minDelay: number;
  maxDelay: number;
  highPriQueMaxBuffTime: number;
  lowPriQueMaxBuffTime: number;
  highPriQueBuffPktNums: number;
  lowPriQueBuffPktNums: number;
  ulPdcpRate: number;
  dlPdcpRate: number;
  ulDiscardCnt: number;
  dlDiscardCnt: number;
  timestamp1: number;
  timestamp2: number;
}

const URC_KEYWORDS = [
  '^PDCPDATAINFO:', '^RSSI:', '^CERSSI:', '^HCSQ:', 'RING',
  '^ANLEVEL:', '^AUDEND:', '+CBM:', '+CBMI:', '+CCWA:', '+CDS:', '+CDSI:',
  '^CEND:', '+CEREG:', '+CGREG:', '+CLIP:', '+CMT:', '+CMTI:', '^CONF:',
  '^CONN:', '^CPBREADY:', '+CREG:', '^CRSSI:', '^CSNR:', '+CSSI:', '+CSSU:',
  '+CTZV:', '+CUSATEND:', '+CUSATP:', '+C5GREG:', '^DATASETRULT:', '^DSDORMANT:',
  '^DSFLOWRPT:', '^ECLREC:', '^EFSSTATE:', '^ERRRPT:', '^FOTASTATE:', '^FWLSTATE:',
  '^MODE:', '^NDISSTAT:', '^NISMSFWD:', '^NWNAME:', '^NWTIME:',
  '^ORIG:', '^RFSWITCH:', '^RSSILVL:', '^SIMRESET:', '^SIMST:', '^SMMEMFULL:',
  '^SRVST:', '^STIN:', '^SUPLCONN:', '^THERM:', '^THERMEX:', '^WAKEUPIN:',
  '+XADPCLKFREQINFO:', '^XDSTATUS:', '+XTS:', '^USIMMEX:', '^USIMICCID:',
  '^LCACELLURC:', '^PLMN:', '^IMSSRVSTATUS:', '^DCONN:', '^DSAMBR:', '^REJINFO',
  '+CUSD:', '^LENDC:',
];

export function isUnsolicitedText(text: string): boolean {
  return URC_KEYWORDS.some((keyword) => text.includes(keyword));
}
