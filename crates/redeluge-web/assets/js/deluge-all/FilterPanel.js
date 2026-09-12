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
                fields: ['filter', 'count'],
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
    },
});

// The tracker filter had the tracker's own favicon here, served by Deluge's
// icon fetcher. redeluge has none, so the URL answered 404 once per host on
// every sidebar refresh and drew nothing. The indent stays, which keeps the
// tracker list lined up with the state and label lists beside it.
Deluge.FilterPanel.templates = {
    tracker_host:
        '<div class="x-deluge-filter">{filter:htmlEncode} ({count})</div>',
};
