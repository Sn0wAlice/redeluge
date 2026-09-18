/**
 * Deluge.FilterPanel.js
 *
 * Copyright (c) Damien Churchill 2009-2010 <damoxc@gmail.com>
 *
 * This file is part of Deluge and is licensed under GNU General Public License 3.0, or later, with
 * the additional special exception to link portions of this program with the OpenSSL library.
 * See LICENSE for more details.
 */
Ext.ns('Deluge');

/**
 * Turns a filter value into something that is safe as a class name.
 *
 * The sidebar builds an icon class out of the filter it is showing. For the
 * state list those values are a fixed vocabulary, but the label and owner
 * lists carry whatever someone typed, and that went into a `class="..."`
 * attribute unquoted-safe: one apostrophe in a label ended the attribute and
 * the rest of the row rendered as markup.
 */
Ext.util.Format.delugeFilterClass = function (value) {
    return String(value)
        .toLowerCase()
        .replace(/[^a-z0-9_-]/g, '-');
};

/**
 * @class Deluge.FilterPanel
 * @extends Ext.list.ListView
 */
Deluge.FilterPanel = Ext.extend(Ext.Panel, {
    autoScroll: true,

    border: false,

    show_zero: null,

    initComponent: function () {
        Deluge.FilterPanel.superclass.initComponent.call(this);
        this.filterType = this.initialConfig.filter;
        var title = '';
        if (this.filterType == 'state') {
            title = _('States');
        } else if (this.filterType == 'tracker_host') {
            title = _('Trackers');
        } else if (this.filterType == 'owner') {
            title = _('Owner');
        } else if (this.filterType == 'label') {
            title = _('Labels');
        } else {
            (title = this.filterType.replace('_', ' ')),
                (parts = title.split(' ')),
                (title = '');
            Ext.each(parts, function (p) {
                fl = p.substring(0, 1).toUpperCase();
                title += fl + p.substring(1) + ' ';
            });
        }
        this.setTitle(_(title));

        if (Deluge.FilterPanel.templates[this.filterType]) {
            var tpl = Deluge.FilterPanel.templates[this.filterType];
        } else {
            var tpl =
                '<div class="x-deluge-filter x-deluge-{filter:delugeFilterClass}">{filter:htmlEncode} ({count})</div>';
        }

        this.list = this.add({
            xtype: 'listview',
            singleSelect: true,
            hideHeaders: true,
            reserveScrollOffset: true,
            store: new Ext.data.ArrayStore({
                idIndex: 0,
                // `healthCls` is the tracker list's: the colour a row is drawn
                // in when its tracker is failing. Empty everywhere else, which
                // is what the other templates ask for.
                fields: ['filter', 'count', 'healthCls'],
            }),
            columns: [
                {
                    id: 'filter',
                    sortable: false,
                    tpl: tpl,
                    dataIndex: 'filter',
                },
            ],
        });
        this.relayEvents(this.list, ['selectionchange']);

        // Right-clicking a tracker or a label opens what that group applies to
        // its torrents; right-clicking Unregistered offers to throw the whole
        // group away. The other lists have nothing to offer: an owner is not
        // something this daemon lets you set rules on.
        if (
            this.filterType == 'tracker_host' ||
            this.filterType == 'label' ||
            this.filterType == 'state'
        ) {
            this.list.on('contextmenu', this.onContextMenu, this);
        }
    },

    /**
     * The menu for one row, where that row has one.
     *
     * The row is not selected on the way, deliberately: selecting one filters
     * the torrent list, and a right-click that quietly changed what the list
     * is showing would be a side effect nobody asked for.
     */
    onContextMenu: function (view, index, node, e) {
        // The browser's own menu over a sidebar row offers nothing useful, and
        // it is in the way whether or not this row has a menu of its own.
        e.stopEvent();

        var record = this.getStore().getAt(index);
        if (!record) return;
        var value = record.id;

        if (this.filterType == 'state') {
            // One row in this list has anything to offer: the torrents their
            // tracker has stopped recognising, which is a group that exists to
            // be thrown away.
            if (value != 'Unregistered' || !record.get('count')) return;
            this.showMenu(e, 'unregistered');
            return;
        }

        // `All` is every tracker or label at once, and the empty row is the
        // torrents that have none. Neither is something a rule can be set on.
        if (!value || value == 'All') return;
        this.menuHost = value;
        this.showMenu(e, this.filterType == 'label' ? 'label' : 'tracker');
    },

    /**
     * Builds the menu this row wants, once, and shows it where the mouse is.
     */
    showMenu: function (e, kind) {
        if (!this.menus) this.menus = {};
        if (!this.menus[kind]) {
            if (kind == 'unregistered') {
                this.menus[kind] = new Ext.menu.Menu({
                    items: [
                        {
                            text: _('Remove these torrents...'),
                            iconCls: 'icon-remove',
                            handler: this.onRemoveUnregistered,
                            scope: this,
                        },
                    ],
                });
            } else if (kind == 'label') {
                // The same window the Edit button in Preferences opens, on the
                // row the label already has here: a label's settings belong to
                // the label, not to the page that happens to list it.
                this.menus[kind] = new Ext.menu.Menu({
                    items: [
                        {
                            text: _('Settings...'),
                            iconCls: 'x-deluge-preferences',
                            handler: this.onLabelSettingsClick,
                            scope: this,
                        },
                    ],
                });
            } else {
                this.menus[kind] = new Ext.menu.Menu({
                    items: [
                        // Above Settings, because looking is what you do first
                        // and changing what happens to a hundred torrents is
                        // what you do after.
                        {
                            text: _('Info...'),
                            iconCls: 'icon-tracker-info',
                            handler: this.onInfoClick,
                            scope: this,
                        },
                        {
                            text: _('Settings...'),
                            iconCls: 'x-deluge-preferences',
                            handler: this.onSettingsClick,
                            scope: this,
                        },
                    ],
                });
            }
        }
        this.menus[kind].showAt(e.getXY());
    },

    /**
     * Hands the whole unregistered group to the ordinary Remove dialog.
     *
     * The same dialog as the toolbar's Remove, so the choice between keeping
     * the files and deleting them is made in the one place that has always
     * asked it, and nothing here can delete anything on its own.
     */
    onRemoveUnregistered: function () {
        deluge.client.core.get_torrents_status(
            { state: 'Unregistered' },
            ['name'],
            {
                success: function (torrents) {
                    var ids = [];
                    for (var id in torrents || {}) {
                        ids.push(id);
                    }
                    if (!ids.length) {
                        // The group emptied itself between the right-click and
                        // the answer, which a re-announce can do.
                        deluge.ui.update();
                        return;
                    }
                    deluge.removeWindow.show(ids);
                },
                scope: this,
            }
        );
    },

    // The menus are not items of this panel, so they are not taken with it.
    // The sidebar drops every panel on a disconnect, which happens as often as
    // somebody's network does.
    onDestroy: function () {
        for (var kind in this.menus || {}) {
            this.menus[kind].destroy();
        }
        this.menus = null;
        Deluge.FilterPanel.superclass.onDestroy.call(this);
    },

    onLabelSettingsClick: function () {
        if (!this.menuHost) return;
        Deluge.LabelSettingsWindow.open(this.menuHost);
    },

    onInfoClick: function () {
        if (!this.menuHost) return;
        // Built when it is first wanted rather than with the interface, the
        // same as the settings window below: most sessions never open either.
        if (!deluge.trackerInfo) {
            deluge.trackerInfo = new Deluge.TrackerInfoWindow();
        }
        deluge.trackerInfo.show(this.menuHost);
    },

    onSettingsClick: function () {
        if (!this.menuHost) return;
        // Built when it is first wanted rather than with the interface: most
        // sessions never open it.
        if (!deluge.trackerSettings) {
            deluge.trackerSettings = new Deluge.TrackerSettingsWindow();
        }
        deluge.trackerSettings.show(this.menuHost);
    },

    /**
     * What to call the group of torrents that have no value for this filter.
     */
    emptyLabel: function () {
        if (this.filterType == 'label') return _('No Label');
        if (this.filterType == 'owner') return _('No Owner');
        if (this.filterType == 'tracker_host') return _('No Tracker');
        return _('None');
    },

    /**
     * Return the currently selected filter state
     * @returns {String} the current filter state
     */
    getState: function () {
        if (!this.list.getSelectionCount()) return;

        var state = this.list.getSelectedRecords()[0];
        if (!state) return;
        if (state.id == 'All') return;
        return state.id;
    },

    /**
     * Return the current states in the filter
     */
    getStates: function () {
        return this.states;
    },

    /**
     * Return the Store for the ListView of the FilterPanel
     * @returns {Ext.data.Store} the ListView store
     */
    getStore: function () {
        return this.list.getStore();
    },

    /**
     * Update the states in the FilterPanel
     */
    updateStates: function (states) {
        this.states = {};
        Ext.each(
            states,
            function (state) {
                this.states[state[0]] = state[1];
            },
            this
        );

        var show_zero =
            this.show_zero == null
                ? deluge.config.sidebar_show_zero
                : this.show_zero;
        if (!show_zero) {
            var newStates = [];
            Ext.each(states, function (state) {
                if (state[1] > 0 || state[0] == 'All') {
                    newStates.push(state);
                }
            });
            states = newStates;
        }

        var store = this.getStore();
        var filters = {};
        Ext.each(
            states,
            function (s, i) {
                var record = store.getById(s[0]);
                if (!record) {
                    record = new store.recordType({
                        filter: s[0],
                        count: s[1],
                    });
                    record.id = s[0];
                    store.insert(i, record);
                }
                record.beginEdit();
                // An empty value is a real group: the torrents with no label,
                // or with no owner. It drew as a blank row with a count beside
                // it and nothing to say what it was.
                record.set('filter', s[0] === '' ? this.emptyLabel() : _(s[0]));
                record.set('count', s[1]);
                record.set('healthCls', this.healthClass(s[0]));
                record.endEdit();
                filters[s[0]] = true;
            },
            this
        );

        store.each(function (record) {
            if (filters[record.id]) return;
            store.remove(record);
            var selected = this.list.getSelectedRecords()[0];
            if (!selected) return;
            if (selected.id == record.id) {
                this.list.select(0);
            }
        }, this);

        store.commitChanges();

        if (!this.list.getSelectionCount()) {
            this.list.select(0);
        }

        // Rides the sidebar's own refresh rather than keeping a timer: there is
        // nothing to colour when the sidebar is not being drawn, and the
        // throttle inside keeps it off the two-second poll.
        this.refreshHealth();
    },

    /**
     * The class one tracker row is drawn with, from the last health answer.
     *
     * Only the tracker list has these, and only for rows that are a tracker:
     * `All` is every tracker at once and the empty row is the torrents that
     * have none, so neither has a state of its own to show.
     */
    healthClass: function (value) {
        if (this.filterType != 'tracker_host') return '';
        if (!value || value == 'All') return '';
        var health = (this.health || {})[value];
        if (!health) return '';
        return 'x-deluge-tracker-health x-deluge-tracker-' + health;
    },

    /**
     * Asks the daemon which trackers are answering, at most every so often.
     *
     * The answer is one word per domain and costs a walk of every torrent's
     * tracker list, which is cheap but not two-seconds cheap, and a tracker
     * that has just gone down is not news that has to arrive within a poll.
     */
    refreshHealth: function () {
        if (this.filterType != 'tracker_host') return;
        if (this.healthInFlight) return;
        var now = new Date().getTime();
        if (this.healthAt && now - this.healthAt < Deluge.FilterPanel.HEALTH_INTERVAL) {
            return;
        }
        // Set before the call rather than after it, so a daemon that is slow
        // or gone is asked at the same rate as one that answers.
        this.healthAt = now;
        this.healthInFlight = true;

        deluge.client.redeluge.get_tracker_health({
            success: function (health) {
                this.healthInFlight = false;
                this.health = health || {};
                this.applyHealth();
            },
            failure: function () {
                this.healthInFlight = false;
            },
            scope: this,
        });
    },

    /**
     * Repaints the rows the sidebar already has with the answer that just
     * arrived, without waiting for the next refresh to rebuild them.
     */
    applyHealth: function () {
        if (!this.list || !this.list.getStore()) return;
        var store = this.getStore();
        store.each(function (record) {
            var health = this.healthClass(record.id);
            if (record.get('healthCls') == health) return;
            record.set('healthCls', health);
        }, this);
        store.commitChanges();
    },
});

/**
 * How often the tracker rows ask whether their trackers still answer, in
 * milliseconds.
 */
Deluge.FilterPanel.HEALTH_INTERVAL = 20000;

// The tracker filter had the tracker's own favicon here, served by Deluge's
// icon fetcher. redeluge has none, so the URL answered 404 once per host on
// every sidebar refresh and drew nothing. The indent stays, which keeps the
// tracker list lined up with the state and label lists beside it.
// The tracker's state took that indent over: a dot, in the four colours
// `redeluge.get_tracker_health` answers with, drawn where the favicon used to
// be. The class is computed rather than templated because `All` and the
// no-tracker row are not trackers and have no state to show.
Deluge.FilterPanel.templates = {
    tracker_host:
        '<div class="x-deluge-filter {healthCls}">{filter:htmlEncode} ({count})</div>',
};
