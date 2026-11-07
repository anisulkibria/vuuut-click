/** Optional performance addon. Load after tracker.js; consent remains controlled by the tracker. */
import {onCLS,onINP,onLCP,onFCP,onTTFB,type Metric} from 'web-vitals';
const initialPage = location.origin + location.pathname;
const report = (metric: Metric) => window.vuuutClick?.trackVital(metric.name,metric.value,metric.id,initialPage);
if (window.vuuutClick) {onCLS(report);onINP(report);onLCP(report);onFCP(report);onTTFB(report);}
else {console.warn('Vuuut Click: load tracker.js before the optional vitals.js addon');}
