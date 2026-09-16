/**
 * Deluge.ActivityWindow.js
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * What the daemon did without being asked.
 *
 * Six things here act on their own: the share-ratio rule, the idle rule, the
 * disk-space rule, the schedule, and a tracker's rules for labelling, moving
 * and removing. Two of them move or delete files. Until this window existed
 * the only trace any of them left was a line in the daemon's log, which on a
 * container install means knowing to run `docker logs`.
 *
 * So it is not a log viewer. It answers the two questions somebody actually
 * asks once automatic rules are turned on — why is that paused, and what
 * happened to that download — and it answers them where the torrents are.
 *
 * The history lives in the daemon, in memory, newest first:
 * `redeluge.get_recent_actions`.
 */
Ext.ns('Deluge');

/**
 * @class Deluge.ActivityWindow
 * @extends Ext.Window
 */
Deluge.ActivityWindow = Ext.extend(Ext.Window, {
    title: _('Activity'),
    width: 720,
    height: 420,
    layout: 'fit',
    closeAction: 'hide',
    constrainHeader: true,
    plain: true,
    minWidth: 480,
    minHeight: 260,

    /** How often the window refreshes itself while it is open, in seconds. */
    interval: 15,

    initComponent: function () {
        Deluge.ActivityWindow.superclass.initComponent.call(this);

        // No `idIndex`: it names which column is the id, it does not add one,
        // so declaring position 0 as the id while the rows also carried one
        // shifted every field by a column.
        this.store = new Ext.data.ArrayStore({
            fields: [
                { name: 'time', type: 'float' },
                { name: 'rule', type: 'string' },
                { name: 'did', type: 'string' },
                { name: 'name', type: 'string' },
                { name: 'detail', type: 'string' },
            ],
        });

        this.grid = this.add({
            xtype: 'grid',
            store: this.store,
            border: false,
            autoExpandColumn: 'detail',
            viewConfig: {
                emptyText: _(
                    'Nothing yet. This fills in when a rule pauses, moves, labels or removes something on its own.'
                ),
                deferEmptyText: false,
            },
            columns: [
                {
                    header: _('When'),
                    dataIndex: 'time',
                    width: 130,
                    renderer: this.renderWhen,
                },
                {
                    header: _('Rule'),
                    dataIndex: 'rule',
                    width: 80,
                    renderer: this.renderRule,
                },
                {
                    header: _('Did'),
                    dataIndex: 'did',
                    width: 80,
                    renderer: this.renderDid,
                },
                {
                    header: _('Torrent'),
                    dataIndex: 'name',
                    width: 200,
                    renderer: Ext.util.Format.htmlEncode,
                },
                {
                    id: 'detail',
                    header: _('Why'),
                    dataIndex: 'detail',
                    renderer: Ext.util.Format.htmlEncode,
                },
            ],
        });

        this.addButton(_('Refresh'), this.load, this);
        this.addButton(_('Close'), this.onClose, this);

        this.on('show', this.onShown, this);
        this.on('hide', this.stopPolling, this);
        this.on('destroy', this.stopPolling, this);
    },

    onShown: function () {
        this.load();
        // Only while it is open: a window nobody is looking at has no business
        // asking the daemon anything.
        this.stopPolling();
        this.timer = window.setInterval(
            this.load.createDelegate(this),
            this.interval * 1000
        );
    },

    stopPolling: function () {
        if (!this.timer) return;
        window.clearInterval(this.timer);
        this.timer = null;
    },

    onClose: function () {
        this.hide();
    },

    load: function () {
        deluge.client.redeluge.get_recent_actions(200, {
            success: function (actions) {
                if (!this.isVisible()) return;
                var rows = [];
                Ext.each(actions || [], function (action) {
                    rows.push([
                        action['time'],
                        action['rule'],
                        action['did'],
                        action['name'] || action['torrent_id'] || '',
                        action['detail'] || '',
                    ]);
                });
                this.store.loadData(rows);
            },
            failure: function () {
                // The daemon may have gone. An empty list says as much as an
                // error dialog would, over a window somebody left open.
                if (!this.isVisible()) return;
                this.store.removeAll();
            },
            scope: this,
        });
    },

    /**
     * How long ago, because that is what is being asked.
     *
     * "4 minutes ago" answers "is this why it just stopped"; a timestamp makes
     * you do the subtraction. Anything older than a day gets the date, where
     * the subtraction stops being worth doing.
     */
    renderWhen: function (value) {
        var seconds = Math.round(new Date().getTime() / 1000 - Number(value));
        if (!value) return '';
        if (seconds < 5) return _('just now');
        if (seconds < 86400) {
            return String.format(_('{0} ago'), ftime(seconds));
        }
        return fdate(value);
    },

    /**
     * Which rule did it, in words rather than the daemon's key.
     */
    renderRule: function (value) {
        var names = {
            tracker: _('Tracker'),
            idle: _('Idle'),
            disk: _('Disk'),
            schedule: _('Schedule'),
            ratio: _('Ratio'),
        };
        return names[value] || Ext.util.Format.htmlEncode(value);
    },

    renderDid: function (value) {
        var names = {
            labelled: _('Labelled'),
            moved: _('Moved'),
            removed: _('Removed'),
            paused: _('Paused'),
            resumed: _('Resumed'),
            changed: _('Changed'),
        };
        return names[value] || Ext.util.Format.htmlEncode(value);
    },
});
