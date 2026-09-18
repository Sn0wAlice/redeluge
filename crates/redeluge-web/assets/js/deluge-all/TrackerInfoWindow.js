/**
 * Deluge.TrackerInfoWindow.js
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * What the trackers of one sidebar row are actually doing.
 *
 * The row is a domain, not a tracker: the sidebar groups `tracker.example.org`
 * and `backup.example.org` together, because that is the unit a set of rules
 * belongs to. Their state is not shared, though. One can be refusing every
 * announce while the other answers, and until this window existed the only
 * trace of that was the tracker column of whichever torrent happened to be on
 * the failing one — a tracker down for a week looked exactly like a torrent
 * nobody is seeding.
 *
 * So the row is taken apart again here: a summary of the domain, and a tab per
 * announce URL under it, each saying whether that tracker answers and what it
 * said when it did not.
 *
 * Everything shown comes from `redeluge.get_tracker_info`, which reports what
 * libtorrent already knew. Opening a window does not announce and does not
 * scrape: a few hundred requests is not a reasonable thing to do to a tracker
 * because somebody was curious.
 */
Ext.ns('Deluge');

/**
 * @class Deluge.TrackerInfoWindow
 * @extends Ext.Window
 */
Deluge.TrackerInfoWindow = Ext.extend(Ext.Window, {
    title: _('Tracker Info'),
    width: 620,
    height: 540,
    layout: 'fit',
    buttonAlign: 'right',
    closeAction: 'hide',
    constrainHeader: true,
    plain: true,
    minWidth: 420,
    minHeight: 320,

    /** How often it refreshes itself while it is open, in seconds. */
    interval: 15,

    initComponent: function () {
        Deluge.TrackerInfoWindow.superclass.initComponent.call(this);

        this.addButton(_('Refresh'), this.load, this);
        this.addButton(_('Close'), this.hide, this);

        this.tabs = this.add({
            xtype: 'tabpanel',
            activeTab: 0,
            border: false,
            deferredRender: false,
            items: [],
        });

        // A window that is hidden rather than destroyed keeps its timer
        // otherwise, and goes on polling the daemon for a tracker nobody is
        // looking at.
        this.on('hide', this.stopTimer, this);
        this.on('destroy', this.stopTimer, this);
    },

    /**
     * Opens the window on one domain, the string the sidebar row carries.
     */
    show: function (host) {
        Deluge.TrackerInfoWindow.superclass.show.call(this);
        this.host = host;
        this.setTitle(String.format(_('Tracker Info: {0}'), host));

        // Blank while the answer is on its way, rather than the previous
        // tracker's figures sitting there looking like this one's.
        this.tabs.removeAll(true);
        this.tabs.add({
            title: _('Loading'),
            bodyStyle: 'padding: 10px',
            html: Ext.util.Format.htmlEncode(_('Asking the daemon...')),
        });
        this.tabs.setActiveTab(0);
        this.doLayout();

        this.load();
        this.startTimer();
    },

    startTimer: function () {
        this.stopTimer();
        this.timer = setInterval(
            this.load.createDelegate(this),
            this.interval * 1000
        );
    },

    stopTimer: function () {
        if (!this.timer) return;
        clearInterval(this.timer);
        this.timer = null;
    },

    load: function () {
        if (!this.host) return;
        deluge.client.redeluge.get_tracker_info(this.host, {
            success: this.onInfo,
            failure: this.onFailure,
            scope: this,
        });
    },

    onFailure: function () {
        if (!this.isVisible()) return;
        // The daemon may have gone away. An empty window beats an error dialog
        // over something somebody only opened to look at.
        this.tabs.removeAll(true);
        this.tabs.add({
            title: _('Tracker'),
            bodyStyle: 'padding: 10px',
            html: Ext.util.Format.htmlEncode(
                _('The daemon did not answer. It may not be connected.')
            ),
        });
        this.tabs.setActiveTab(0);
        this.doLayout();
    },

    onInfo: function (info) {
        if (!this.isVisible() || !info) return;

        // Which tab was being read, so a refresh every fifteen seconds does not
        // throw somebody back to the summary mid-sentence.
        var previous = this.tabs.getActiveTab();
        var wanted = previous ? previous.trackerUrl : null;

        this.tabs.removeAll(true);
        this.tabs.add(this.summaryPanel(info));

        var trackers = info.trackers || [];
        var titles = this.titlesFor(trackers);
        Ext.each(
            trackers,
            function (tracker, index) {
                this.tabs.add(this.trackerPanel(tracker, titles[index]));
            },
            this
        );

        var found = 0;
        if (wanted) {
            Ext.each(trackers, function (tracker, index) {
                if (tracker.url == wanted) found = index + 1;
            });
        }
        this.tabs.setActiveTab(found);
        this.doLayout();
    },

    /**
     * A name for each tab.
     *
     * The host, which is what tells two trackers of one domain apart. It is not
     * always enough: an HTTP and a UDP announce on the same machine share a
     * host, and so do two paths on one tracker — a private tracker's per-user
     * announce URLs are the usual case. Where the host repeats, the whole URL
     * is used, minus the scheme, because any shorter rule has a pair it cannot
     * tell apart and two tabs with one name is worse than one long name.
     */
    titlesFor: function (trackers) {
        var seen = {};
        Ext.each(trackers, function (tracker) {
            seen[tracker.host] = (seen[tracker.host] || 0) + 1;
        });
        return trackers.map(function (tracker) {
            if (seen[tracker.host] < 2) return tracker.host || _('Unknown');
            var parts = String(tracker.url).split('://');
            return parts.length > 1 ? parts.slice(1).join('://') : tracker.url;
        });
    },

    summaryPanel: function (info) {
        var rows = '';
        rows += this.row(_('Trackers'), String((info.trackers || []).length));
        rows += this.row(_('Torrents'), String(info.torrents || 0));
        rows += this.row(_('States'), this.states(info.states));
        rows += this.row(
            _('Swarm'),
            String.format(
                _('{0} seeds, {1} peers'),
                info.seeds || 0,
                info.peers || 0
            )
        );
        rows += this.row(_('Size'), fsize(info.size || 0));
        rows += this.transfer(info);
        if (info.unregistered) {
            rows += this.row(
                _('Unregistered'),
                this.bad(
                    String.format(
                        _('{0} torrents this tracker no longer recognises'),
                        info.unregistered
                    )
                )
            );
        }

        return {
            title: _('Summary'),
            trackerUrl: null,
            autoScroll: true,
            bodyStyle: 'padding: 10px',
            html:
                this.banner(info, String.format(_('All of {0}'), info.host)) +
                '<table class="x-deluge-tracker-info">' +
                rows +
                '</table>',
        };
    },

    trackerPanel: function (tracker, title) {
        var rows = '';
        rows += this.row(_('URL'), this.code(tracker.url));
        rows += this.row(_('Tier'), String(tracker.tier || 0));
        rows += this.row(
            _('Torrents'),
            tracker.torrents == tracker.announcing
                ? String(tracker.torrents || 0)
                : String.format(
                      _('{0} listed, {1} announcing here'),
                      tracker.torrents || 0,
                      tracker.announcing || 0
                  )
        );
        rows += this.row(_('States'), this.states(tracker.states));

        // The swarm came back from an announce, so it belongs to the tracker
        // that answered. A listed backup nobody announces to has none, and a
        // zero there would read as an empty swarm rather than as no answer.
        rows += this.row(
            _('Swarm'),
            tracker.announcing
                ? String.format(
                      _('{0} seeds, {1} peers'),
                      tracker.seeds || 0,
                      tracker.peers || 0
                  )
                : this.quiet(_('nothing announces here, so it has said nothing'))
        );
        rows += this.row(
            _('Next announce'),
            tracker.next_announce
                ? ftime(tracker.next_announce)
                : this.quiet(_('none due'))
        );
        rows += this.row(_('Size'), fsize(tracker.size || 0));
        rows += this.transfer(tracker);
        if (tracker.unregistered) {
            rows += this.row(
                _('Unregistered'),
                this.bad(
                    String.format(
                        _('{0} torrents this tracker no longer recognises'),
                        tracker.unregistered
                    )
                )
            );
        }

        return {
            title: title,
            trackerUrl: tracker.url,
            autoScroll: true,
            bodyStyle: 'padding: 10px',
            html:
                this.banner(tracker, tracker.url) +
                '<table class="x-deluge-tracker-info">' +
                rows +
                '</table>',
        };
    },

    /**
     * The one thing this window is for, at the top where it cannot be missed:
     * whether this tracker is answering, and what it said when it stopped.
     */
    banner: function (entry, subject) {
        var health = Deluge.TrackerInfoWindow.HEALTH[entry.health] ||
            Deluge.TrackerInfoWindow.HEALTH.unknown;

        var detail = '';
        if (entry.failing) {
            detail = String.format(
                _('{0} of {1} announces are failing'),
                entry.failing,
                entry.failing + entry.working
            );
            if (entry.fails > 1) {
                detail += String.format(_(', {0} tries deep'), entry.fails);
            }
        } else if (entry.updating) {
            detail = _('an announce is in flight');
        }

        var html =
            '<div class="x-deluge-tracker-banner x-deluge-tracker-' +
            health.cls +
            '">' +
            '<span class="x-deluge-tracker-state">' +
            Ext.util.Format.htmlEncode(health.text) +
            '</span>' +
            '<span class="x-deluge-tracker-subject">' +
            Ext.util.Format.htmlEncode(subject || '') +
            '</span>';
        if (detail) {
            html +=
                '<div class="x-deluge-tracker-detail">' +
                Ext.util.Format.htmlEncode(detail) +
                '</div>';
        }
        // What the tracker itself said. Shown verbatim, because the exact
        // sentence is the thing worth searching for when it is unfamiliar.
        if (entry.message) {
            html +=
                '<div class="x-deluge-tracker-message">' +
                Ext.util.Format.htmlEncode(entry.message) +
                '</div>';
        }
        return html + '</div>';
    },

    /**
     * What the torrents of this tracker are doing, as one line.
     */
    states: function (states) {
        if (!states) return this.quiet(_('none'));
        var parts = [];
        Ext.each(
            Deluge.TrackerInfoWindow.STATE_ORDER,
            function (state) {
                if (!states[state]) return;
                parts.push(
                    Ext.util.Format.htmlEncode(_(state)) + ' ' + states[state]
                );
            },
            this
        );
        // Anything the daemon knows about and this list does not, rather than
        // silently dropping it.
        Ext.each(Ext.keys(states), function (state) {
            if (Deluge.TrackerInfoWindow.STATE_ORDER.indexOf(state) > -1) return;
            parts.push(Ext.util.Format.htmlEncode(_(state)) + ' ' + states[state]);
        });
        return parts.length ? parts.join(' &middot; ') : this.quiet(_('none'));
    },

    /**
     * Downloaded, uploaded and the ratio between them, for the torrents of one
     * tracker. The ratio is the number most trackers actually ask about.
     */
    transfer: function (entry) {
        var down = entry.downloaded || 0;
        var up = entry.uploaded || 0;
        var ratio = down > 0 ? (up / down).toFixed(3) : _('∞');
        if (down <= 0 && up <= 0) ratio = '—';
        return (
            this.row(_('Downloaded'), fsize(down)) +
            this.row(_('Uploaded'), fsize(up)) +
            this.row(_('Ratio'), String(ratio))
        );
    },

    row: function (label, value) {
        return (
            '<tr><th>' +
            Ext.util.Format.htmlEncode(label) +
            '</th><td>' +
            value +
            '</td></tr>'
        );
    },

    code: function (text) {
        return (
            '<span class="x-deluge-tracker-url">' +
            Ext.util.Format.htmlEncode(text || '') +
            '</span>'
        );
    },

    bad: function (text) {
        return (
            '<span class="x-deluge-tracker-bad">' +
            Ext.util.Format.htmlEncode(text) +
            '</span>'
        );
    },

    quiet: function (text) {
        return (
            '<span class="x-deluge-tracker-quiet">' +
            Ext.util.Format.htmlEncode(text) +
            '</span>'
        );
    },
});

/**
 * The four states a tracker can be in, as the daemon names them.
 *
 * `down` is reserved for one that is failing everywhere and working nowhere,
 * so the word means what it says. A tracker that is merely failing on some of
 * its torrents is in trouble, which is a different thing to be told.
 */
Deluge.TrackerInfoWindow.HEALTH = {
    ok: { text: _('Working'), cls: 'ok' },
    warning: { text: _('Trouble'), cls: 'warning' },
    down: { text: _('Down'), cls: 'down' },
    unknown: { text: _('Not contacted yet'), cls: 'unknown' },
};

/** The order states are listed in, which is the sidebar's. */
Deluge.TrackerInfoWindow.STATE_ORDER = [
    'Downloading',
    'Seeding',
    'Queued',
    'Paused',
    'Checking',
    'Allocating',
    'Moving',
    'Error',
];
