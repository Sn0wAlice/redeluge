/**
 * Deluge.TorrentGrid.js
 *
 * Copyright (c) Damien Churchill 2009-2010 <damoxc@gmail.com>
 *
 * This file is part of Deluge and is licensed under GNU General Public License 3.0, or later, with
 * the additional special exception to link portions of this program with the OpenSSL library.
 * See LICENSE for more details.
 */

(function () {
    /* Renderers for the Torrent Grid */
    function queueRenderer(value) {
        return value == -1 ? '' : value + 1;
    }
    function torrentNameRenderer(value, p, r) {
        // `state` is what picks the icon. A record without it used to throw
        // here, and a renderer that throws stops the grid's render loop, so
        // one torrent in an unexpected state emptied every row after it.
        var state = String(r.data['state'] || '').toLowerCase();
        return String.format(
            '<div class="torrent-name x-deluge-{0}">{1}</div>',
            state.replace(/[^a-z0-9_-]/g, ''),
            Ext.util.Format.htmlEncode(value || '')
        );
    }
    function torrentSpeedRenderer(value) {
        if (!value) return;
        return fspeed(value);
    }
    function torrentLimitRenderer(value) {
        if (value == -1) return '';
        return fspeed(value * 1024.0);
    }
    function torrentProgressRenderer(value, p, r) {
        value = new Number(value);
        var text = _(r.data['state'] || '') + ' ' + value.toFixed(2) + '%';
        // The old form indexed the result of a regex match without checking
        // it, so a column whose style carried no width threw from inside the
        // grid's render loop and left the rows it had not reached blank.
        var width = Deluge.columnWidth(p, 150);
        return Deluge.progressBar(value, width - 8, text);
    }
    function seedsRenderer(value, p, r) {
        if (r.data['total_seeds'] > -1) {
            return String.format('{0} ({1})', value, r.data['total_seeds']);
        } else {
            return value;
        }
    }
    function peersRenderer(value, p, r) {
        if (r.data['total_peers'] > -1) {
            return String.format('{0} ({1})', value, r.data['total_peers']);
        } else {
            return value;
        }
    }
    function availRenderer(value, p, r) {
        return value < 0 ? '&infin;' : parseFloat(new Number(value).toFixed(3));
    }
    function trackerRenderer(value, p, r) {
        // Deluge drew the tracker's own favicon here, fetched by the web
        // server from each tracker and cached. redeluge has no such fetcher:
        // it would mean the server making an outbound request to every host a
        // torrent names, which is not something an interface should do
        // quietly. Without one the per-host URL answered 404 for every row on
        // every refresh, leaving a twenty-pixel indent where the icon was, so
        // the column is the host name and nothing else.
        return Ext.util.Format.htmlEncode(value || '');
    }

    /**
     * What the idle rule is about to do to this torrent.
     *
     * Counted from the two timestamps rather than from a sentence the server
     * wrote, so it ticks between polls instead of being two seconds stale.
     */
    function idleRenderer(value, p, r) {
        var now = new Date().getTime() / 1000;
        var resumeAt = Number(r.data['idle_resume_at']) || 0;
        if (resumeAt > now) {
            return String.format(
                _('resumes in {0}'),
                ftime(Math.round(resumeAt - now))
            );
        }
        var pauseAt = Number(r.data['idle_pause_at']) || 0;
        if (pauseAt > now) {
            return String.format(
                _('pauses in {0}'),
                ftime(Math.round(pauseAt - now))
            );
        }
        // Idle, past its grace, and waiting only for something to queue up
        // behind it. Saying "now" would be wrong: it may never happen.
        if (pauseAt > 0) {
            return _('idle');
        }
        return '';
    }

    function etaSorter(eta) {
        if (eta === 0) return Number.MAX_VALUE;
        if (eta <= -1) return Number.MAX_SAFE_INTEGER;
        return eta;
    }

    function dateOrNever(date) {
        return date > 0.0 ? fdate(date) : _('Never');
    }

    function timeOrInf(time) {
        if (time === 0) return '';
        if (time <= -1) return '&infin;';
        return ftime(time);
    }

    /**
     * Deluge.TorrentGrid Class
     *
     * @author Damien Churchill <damoxc@gmail.com>
     * @version 1.3
     *
     * @class Deluge.TorrentGrid
     * @extends Ext.grid.GridPanel
     * @constructor
     * @param {Object} config Configuration options
     */
    Deluge.TorrentGrid = Ext.extend(Ext.grid.GridPanel, {
        // object to store contained torrent ids
        torrents: {},

        columns: [
            {
                id: 'queue',
                header: '#',
                width: 30,
                sortable: true,
                renderer: queueRenderer,
                dataIndex: 'queue',
            },
            {
                id: 'name',
                header: _('Name'),
                width: 150,
                sortable: true,
                renderer: torrentNameRenderer,
                dataIndex: 'name',
            },
            {
                header: _('Size'),
                width: 75,
                sortable: true,
                renderer: fsize,
                dataIndex: 'total_wanted',
            },
            {
                header: _('Progress'),
                width: 150,
                sortable: true,
                renderer: torrentProgressRenderer,
                dataIndex: 'progress',
            },
            {
                header: _('Seeds'),
                hidden: true,
                width: 60,
                sortable: true,
                renderer: seedsRenderer,
                dataIndex: 'num_seeds',
            },
            {
                header: _('Peers'),
                hidden: true,
                width: 60,
                sortable: true,
                renderer: peersRenderer,
                dataIndex: 'num_peers',
            },
            {
                header: _('Down Speed'),
                width: 80,
                sortable: true,
                renderer: torrentSpeedRenderer,
                dataIndex: 'download_payload_rate',
            },
            {
                header: _('Up Speed'),
                width: 80,
                sortable: true,
                renderer: torrentSpeedRenderer,
                dataIndex: 'upload_payload_rate',
            },
            {
                header: _('ETA'),
                width: 60,
                sortable: true,
                renderer: timeOrInf,
                dataIndex: 'eta',
            },
            {
                header: _('Ratio'),
                hidden: true,
                width: 60,
                sortable: true,
                renderer: availRenderer,
                dataIndex: 'ratio',
            },
            {
                header: _('Avail'),
                hidden: true,
                width: 60,
                sortable: true,
                renderer: availRenderer,
                dataIndex: 'distributed_copies',
            },
            {
                header: _('Added'),
                hidden: true,
                width: 80,
                sortable: true,
                renderer: fdate,
                dataIndex: 'time_added',
            },
            {
                header: _('Complete Seen'),
                hidden: true,
                width: 80,
                sortable: true,
                renderer: dateOrNever,
                dataIndex: 'last_seen_complete',
            },
            {
                header: _('Completed'),
                hidden: true,
                width: 80,
                sortable: true,
                renderer: dateOrNever,
                dataIndex: 'completed_time',
            },
            {
                header: _('Tracker'),
                hidden: true,
                width: 120,
                sortable: true,
                renderer: trackerRenderer,
                dataIndex: 'tracker_host',
            },
            {
                header: _('Download Folder'),
                hidden: true,
                width: 120,
                sortable: true,
                renderer: fplain,
                dataIndex: 'download_location',
            },
            {
                // The idle rule's countdown, beside ETA because it is the
                // other thing on this row that is a time you are waiting for.
                header: _('Idle'),
                width: 110,
                sortable: true,
                renderer: idleRenderer,
                dataIndex: 'idle_pause_at',
            },
            {
                // Beside Owner, the other thing that puts torrents in groups.
                // Shown by default: a label you cannot see is one you cannot
                // tell apart from no label at all, and the sidebar's Labels
                // list only says how many, not which.
                header: _('Label'),
                width: 90,
                sortable: true,
                renderer: fplain,
                dataIndex: 'label',
            },
            {
                header: _('Owner'),
                width: 80,
                sortable: true,
                renderer: fplain,
                dataIndex: 'owner',
            },
            {
                header: _('Public'),
                hidden: true,
                width: 80,
                sortable: true,
                renderer: fplain,
                dataIndex: 'public',
            },
            {
                header: _('Shared'),
                hidden: true,
                width: 80,
                sortable: true,
                renderer: fplain,
                dataIndex: 'shared',
            },
            {
                header: _('Downloaded'),
                hidden: true,
                width: 75,
                sortable: true,
                renderer: fsize,
                dataIndex: 'total_done',
            },
            {
                header: _('Uploaded'),
                hidden: true,
                width: 75,
                sortable: true,
                renderer: fsize,
                dataIndex: 'total_uploaded',
            },
            {
                header: _('Remaining'),
                hidden: true,
                width: 75,
                sortable: true,
                renderer: fsize,
                dataIndex: 'total_remaining',
            },
            {
                header: _('Down Limit'),
                hidden: true,
                width: 75,
                sortable: true,
                renderer: torrentLimitRenderer,
                dataIndex: 'max_download_speed',
            },
            {
                header: _('Up Limit'),
                hidden: true,
                width: 75,
                sortable: true,
                renderer: torrentLimitRenderer,
                dataIndex: 'max_upload_speed',
            },
            {
                header: _('Seeds:Peers'),
                hidden: true,
                width: 75,
                sortable: true,
                renderer: availRenderer,
                dataIndex: 'seeds_peers_ratio',
            },
            {
                header: _('Last Transfer'),
                hidden: true,
                width: 75,
                sortable: true,
                renderer: ftime,
                dataIndex: 'time_since_transfer',
            },
        ],

        meta: {
            root: 'torrents',
            idProperty: 'id',
            fields: [
                {
                    name: 'queue',
                    sortType: Deluge.data.SortTypes.asQueuePosition,
                },
                { name: 'name', sortType: Deluge.data.SortTypes.asName },
                { name: 'total_wanted', type: 'int' },
                { name: 'state' },
                { name: 'progress', type: 'float' },
                { name: 'num_seeds', type: 'int' },
                { name: 'total_seeds', type: 'int' },
                { name: 'num_peers', type: 'int' },
                { name: 'total_peers', type: 'int' },
                { name: 'download_payload_rate', type: 'int' },
                { name: 'upload_payload_rate', type: 'int' },
                { name: 'eta', type: 'int', sortType: etaSorter },
                { name: 'ratio', type: 'float' },
                { name: 'distributed_copies', type: 'float' },
                { name: 'time_added', type: 'int' },
                { name: 'last_seen_complete', type: 'int' },
                { name: 'completed_time', type: 'int' },
                { name: 'tracker_host' },
                { name: 'download_location' },
                { name: 'total_done', type: 'int' },
                { name: 'total_uploaded', type: 'int' },
                { name: 'total_remaining', type: 'int' },
                { name: 'max_download_speed', type: 'int' },
                { name: 'max_upload_speed', type: 'int' },
                { name: 'seeds_peers_ratio', type: 'float' },
                { name: 'time_since_transfer', type: 'int' },
                { name: 'label' },
                { name: 'owner' },
                { name: 'idle_since', type: 'float' },
                { name: 'idle_pause_at', type: 'float' },
                { name: 'idle_resume_at', type: 'float' },
                { name: 'space_paused', type: 'bool' },
            ],
        },

        keys: [
            {
                key: 'a',
                ctrl: true,
                stopEvent: true,
                handler: function () {
                    deluge.torrents.getSelectionModel().selectAll();
                },
            },
            {
                key: [46],
                stopEvent: true,
                handler: function () {
                    ids = deluge.torrents.getSelectedIds();
                    deluge.removeWindow.show(ids);
                },
            },
        ],

        constructor: function (config) {
            config = Ext.apply(
                {
                    id: 'torrentGrid',
                    store: new Ext.data.JsonStore(this.meta),
                    columns: this.columns,
                    keys: this.keys,
                    region: 'center',
                    cls: 'deluge-torrents',
                    stripeRows: true,
                    autoExpandColumn: 'name',
                    autoExpandMin: 150,
                    deferredRender: false,
                    autoScroll: true,
                    stateful: true,
                    view: new Ext.ux.grid.BufferView({
                        rowHeight: 26,
                        scrollDelay: false,
                    }),
                },
                config
            );
            Deluge.TorrentGrid.superclass.constructor.call(this, config);
        },

        initComponent: function () {
            Deluge.TorrentGrid.superclass.initComponent.call(this);
            deluge.events.on('torrentsRemoved', this.onTorrentsRemoved, this);
            deluge.events.on('disconnect', this.onDisconnect, this);
            deluge.events.on('connect', this.loadHiddenLabels, this);

            // The header menu exists only once the view has rendered.
            this.on('render', this.installLabelHeaderMenu, this, { single: true });

            this.on('rowcontextmenu', function (grid, rowIndex, e) {
                e.stopEvent();
                var selection = grid.getSelectionModel();
                if (!selection.isSelected(rowIndex)) {
                    selection.selectRow(rowIndex);
                }
                // Read the labels now rather than when the interface loaded:
                // another program adds them through the same API, and the one
                // you want is often the one just created.
                deluge.menus.refreshLabelMenu();
                deluge.menus.torrent.showAt(e.getPoint());
            });
        },

        /**
         * Returns the record representing the torrent at the specified index.
         *
         * @param index {int} The row index of the torrent you wish to retrieve.
         * @return {Ext.data.Record} The record representing the torrent.
         */
        getTorrent: function (index) {
            return this.getStore().getAt(index);
        },

        /**
         * Returns the currently selected record.
         * @ return {Array/Ext.data.Record} The record(s) representing the rows
         */
        getSelected: function () {
            return this.getSelectionModel().getSelected();
        },

        /**
         * Returns the currently selected records.
         */
        getSelections: function () {
            return this.getSelectionModel().getSelections();
        },

        /**
         * Return the currently selected torrent id.
         * @return {String} The currently selected id.
         */
        getSelectedId: function () {
            return this.getSelectionModel().getSelected().id;
        },

        /**
         * Return the currently selected torrent ids.
         * @return {Array} The currently selected ids.
         */
        getSelectedIds: function () {
            var ids = [];
            Ext.each(this.getSelectionModel().getSelections(), function (r) {
                ids.push(r.id);
            });
            return ids;
        },

        update: function (torrents, wipe) {
            var store = this.getStore();

            // Kept so unticking a label can redraw without another poll.
            this.lastTorrents = torrents;
            torrents = this.withoutHiddenLabels(torrents);

            // Need to perform a complete reload of the torrent grid.
            if (wipe) {
                store.removeAll();
                this.torrents = {};
            }

            var newTorrents = [];

            // Update and add any new torrents.
            for (var t in torrents) {
                var torrent = torrents[t];

                if (this.torrents[t]) {
                    var record = store.getById(t);
                    record.beginEdit();
                    for (var k in torrent) {
                        if (record.get(k) != torrent[k]) {
                            record.set(k, torrent[k]);
                        }
                    }
                    record.endEdit();
                } else {
                    var record = new Deluge.data.Torrent(torrent);
                    record.id = t;
                    this.torrents[t] = 1;
                    newTorrents.push(record);
                }
            }
            store.add(newTorrents);

            // Remove any torrents that should not be in the store.
            store.each(function (record) {
                if (!torrents[record.id]) {
                    store.remove(record);
                    delete this.torrents[record.id];
                }
            }, this);
            store.commitChanges();

            var sortState = store.getSortState();
            if (!sortState) return;
            store.sort(sortState.field, sortState.direction);
        },

        /**
         * Which labels are hidden from the current view, by name.
         *
         * A view filter, not a query: the torrents are already here, and the
         * sidebar's own label filter picks one label to look at. This is the
         * other question, "everything except these", and answering it in the
         * browser means it is instant and does not argue with the sidebar.
         */
        hiddenLabels: {},

        /**
         * Labels the person has decided about themselves, this session.
         *
         * A label hidden by default that was ticked back on must not be hidden
         * again by the next reconnect, which re-reads the register.
         */
        labelChoices: {},

        /**
         * The same torrents, without the ones whose label is unticked.
         *
         * Filtering the data on the way in rather than filtering the store:
         * the update below adds, edits and removes records by comparing them
         * with what arrived, and a filtered store hides records from that
         * comparison, which would leave rows behind.
         */
        withoutHiddenLabels: function (torrents) {
            var hidden = this.hiddenLabels;
            var any = false;
            for (var name in hidden) {
                if (hidden[name]) {
                    any = true;
                    break;
                }
            }
            if (!any || !torrents) return torrents;

            // Asking for a label in the sidebar outranks hiding it. Without
            // this, clicking a label that is hidden by default would show an
            // empty list, which looks exactly like a label that lost its
            // torrents.
            var asked = null;
            if (deluge.sidebar && deluge.sidebar.getFilterStates) {
                var states = deluge.sidebar.getFilterStates() || {};
                if (states['label'] !== undefined) asked = states['label'];
            }

            var kept = {};
            for (var id in torrents) {
                var label = torrents[id]['label'] || '';
                if (!hidden[label] || label === asked) kept[id] = torrents[id];
            }
            return kept;
        },

        /**
         * Hides the labels whose register entry says to, on connect.
         *
         * The rule belongs to the label rather than to this browser, which is
         * what makes it worth storing: somebody with three thousand torrents
         * wants the noisy label out of the way on every machine they open,
         * not once per machine.
         */
        loadHiddenLabels: function () {
            deluge.client.label.get_config({
                success: function (config) {
                    var labels = (config && config['labels']) || {};
                    var changed = false;
                    for (var name in labels) {
                        if (this.labelChoices[name]) continue;
                        var options = labels[name] || {};
                        if (options['hide_by_default'] && !this.hiddenLabels[name]) {
                            this.hiddenLabels[name] = true;
                            changed = true;
                        }
                    }
                    if (changed && this.lastTorrents) {
                        this.update(this.lastTorrents, true);
                    }
                },
                // A daemon that is not connected yet has no register to read,
                // and this is not worth an error dialog over.
                failure: Ext.emptyFn,
                scope: this,
            });
        },

        /**
         * Shows or hides one label, and redraws from what was last received.
         */
        setLabelHidden: function (name, hidden) {
            this.labelChoices[name] = true;
            if (hidden) {
                this.hiddenLabels[name] = true;
            } else {
                delete this.hiddenLabels[name];
            }
            if (this.lastTorrents) {
                // Wiped, because a label coming back means records that are
                // not in the store at all have to be added again.
                this.update(this.lastTorrents, true);
            }
        },

        showEveryLabel: function () {
            for (var name in this.hiddenLabels) {
                this.labelChoices[name] = true;
            }
            this.hiddenLabels = {};
            if (this.lastTorrents) this.update(this.lastTorrents, true);
        },

        /**
         * Adds the label list to the Label column's header menu.
         *
         * Ext JS gives every column the same header menu, so what it offers
         * has to be decided when it opens: `hdCtxIndex` is the column that was
         * right-clicked.
         */
        installLabelHeaderMenu: function () {
            var grid = this;
            var view = this.getView();
            if (!view || !view.hmenu || view.hmenu.__labelMenu) return;
            view.hmenu.__labelMenu = true;

            view.hmenu.on('beforeshow', function (menu) {
                var existing = menu.items.get('label-filter');
                if (existing) menu.remove(existing);

                // By dataIndex, not by id: only the columns that declare one
                // have a string id, and the rest carry their position, so an
                // id comparison here matched nothing at all.
                var column = grid.getColumnModel().getDataIndex(view.hdCtxIndex);
                if (column !== 'label') return;

                var items = [];
                var states = null;
                var panel = deluge.sidebar && deluge.sidebar.getFilter('label');
                if (panel && panel.getStates) states = panel.getStates();

                for (var name in states || {}) {
                    if (name === 'All') continue;
                    items.push({
                        text: name === '' ? _('No Label') : name,
                        checked: !grid.hiddenLabels[name],
                        labelName: name,
                        hideOnClick: false,
                        checkHandler: function (item, checked) {
                            grid.setLabelHidden(
                                item.initialConfig.labelName,
                                !checked
                            );
                        },
                    });
                }

                if (!items.length) {
                    items.push({ text: _('No labels yet'), disabled: true });
                } else {
                    items.push('-');
                    items.push({
                        text: _('Show all'),
                        handler: function () {
                            grid.showEveryLabel();
                        },
                    });
                }

                menu.add({
                    itemId: 'label-filter',
                    text: _('Show labels'),
                    menu: { items: items },
                });
            });
        },

        // private
        onDisconnect: function () {
            this.getStore().removeAll();
            this.torrents = {};
        },

        // private
        onTorrentsRemoved: function (torrentIds) {
            var selModel = this.getSelectionModel();
            Ext.each(
                torrentIds,
                function (torrentId) {
                    var record = this.getStore().getById(torrentId);
                    if (selModel.isSelected(record)) {
                        selModel.deselectRow(this.getStore().indexOf(record));
                    }
                    this.getStore().remove(record);
                    delete this.torrents[torrentId];
                },
                this
            );
        },
    });
    deluge.torrents = new Deluge.TorrentGrid();
})();
